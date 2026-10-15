#!/usr/bin/env python3
"""Watch Rust sources and restart only the changed development binaries."""

import fcntl
import hashlib
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import time


ROOT = Path(__file__).resolve().parent.parent
DEV = ROOT / "target" / "pinch-dev"


def sources():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "rust-toolchain.toml"]
    paths.extend(p for p in (ROOT / "crates").rglob("*") if p.is_file())
    paths.extend(p for p in (ROOT / ".cargo").rglob("*") if p.is_file())
    return {str(p): hashlib.sha256(p.read_bytes()).digest() for p in paths if p.exists()}


def stop(process):
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def wait_for_engine(process, path):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("development engine exited; see output above")
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(0.2)
            try:
                connection.connect(str(path))
                return
            except OSError:
                time.sleep(0.1)
    raise RuntimeError("development engine did not become ready")


class Preview:
    def __init__(self):
        self.engine = None
        self.ui = None
        self.digests = {}
        self.env = dict(os.environ, PINCH_HOME=str(DEV / "state"))

    def rebuild(self):
        print("\nBuilding development preview…", flush=True)
        if subprocess.run(
            ["cargo", "build", "--locked", "--workspace", "--target-dir", str(ROOT / "target")],
            cwd=ROOT,
        ).returncode:
            print("Build failed; keeping the previous preview. Save to retry.", flush=True)
            return
        digests = {
            name: hashlib.sha256((ROOT / "target/debug" / name).read_bytes()).digest()
            for name in ("pinchd", "pinch-ui", "pinch")
        }
        engine_changed = (
            self.engine is None
            or self.engine.poll() is not None
            or digests["pinchd"] != self.digests.get("pinchd")
        )
        ui_changed = (
            engine_changed
            or self.ui is None
            or self.ui.poll() is not None
            or digests["pinch-ui"] != self.digests.get("pinch-ui")
        )
        if ui_changed:
            stop(self.ui)
            self.ui = None
        if engine_changed:
            stop(self.engine)
            self.engine = None
        # Run private copies: Cargo can replace its outputs without touching a
        # running preview, and failed builds never disturb the last good build.
        for name, digest in digests.items():
            if digest != self.digests.get(name):
                destination = DEV / name
                staging = DEV / (name + ".new")
                shutil.copy2(ROOT / "target/debug" / name, staging)
                staging.replace(destination)
        if engine_changed:
            (DEV / "state/pinchd.sock").unlink(missing_ok=True)
            self.engine = subprocess.Popen([str(DEV / "pinchd"), "--headless"], env=self.env)
            wait_for_engine(self.engine, DEV / "state/pinchd.sock")
        if ui_changed:
            self.ui = subprocess.Popen([str(DEV / "pinch-ui"), "--dev"], env=self.env)
        self.digests = digests
        print("Preview ready. Watching crates/; Ctrl-C stops the preview.", flush=True)

    def close(self):
        stop(self.ui)
        stop(self.engine)


def main():
    def terminate(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, terminate)
    DEV.mkdir(parents=True, exist_ok=True)
    # One watcher per checkout; never terminate another app's processes.
    with (DEV / "watch.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("A development preview is already running for this checkout.")
        preview = Preview()
        try:
            before = sources()
            preview.rebuild()
            while True:
                time.sleep(0.5)
                current = sources()
                if current == before:
                    continue
                # Wait until saves settle, including multi-file formatter edits.
                while current != before:
                    before = current
                    time.sleep(0.3)
                    current = sources()
                preview.rebuild()
        except KeyboardInterrupt:
            print("\nStopping development preview.", flush=True)
        finally:
            preview.close()


if __name__ == "__main__":
    main()

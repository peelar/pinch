#!/usr/bin/env python3
"""Run the bundled Pinch UI against disposable, interactive fixture data."""

import argparse
from copy import deepcopy
import json
import os
from pathlib import Path
import socketserver
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
SCENARIOS = ("process-list", "empty", "notifications-disabled", "updates-available")


class Fixture:
    def __init__(self, scenario="process-list"):
        data = json.loads((ROOT / "scripts/fixtures/process-list.json").read_text())
        self.protocol_version = data["protocol_version"]
        self.snapshot = data["snapshot"]
        self.logs = data["logs"]
        self.epoch_ms = data["epoch_ms"]
        self.process_rules = {p["id"] for p in self.snapshot["processes"]
                              if p["recognized"] and not self.app(p)}
        if scenario == "empty":
            self.snapshot.update(processes=[], decisions=[], recognized=[])
            self.process_rules.clear()
        elif scenario == "notifications-disabled":
            self.snapshot["notifications"] = "disabled"
        elif scenario == "updates-available":
            self.snapshot["updates"].update(enabled=True, can_check=True,
                                           automatic_checks=True, available_version="0.3.0")

    @staticmethod
    def app(process):
        if process["repo"] or process["origin"] != "unmanaged":
            return None
        for value in (process["app_bundle"], process["launcher_app"]):
            if value:
                path = Path(value)
                if path.suffix == ".app" and path.parent in (
                    Path("/Applications"), Path.home() / "Applications"
                ):
                    return value
        return None

    def current(self):
        result = deepcopy(self.snapshot)
        apps = {app["path"] for app in result["recognized"]}
        for process in result["processes"]:
            process["recognized"] = self.app(process) in apps or process["id"] in self.process_rules
        shift = int(time.time() * 1000) - self.epoch_ms

        def rebase(value):
            if isinstance(value, dict):
                return {key: (item + shift if key in (
                    "started_at_ms", "last_output_ms", "quiet_since_ms",
                    "created_at_ms", "recognized_at_ms", "sampled_at_ms"
                ) and item is not None else rebase(item)) for key, item in value.items()}
            if isinstance(value, list):
                return [rebase(item) for item in value]
            return value

        return rebase(result)

    def process(self, target):
        return next((p for p in self.snapshot["processes"]
                     if p["id"] == target or p["name"] == target), None)

    def stop(self, process):
        process.update(status={"state": "exited", "code": None}, metrics=None, ports=[])
        self.snapshot["decisions"] = [d for d in self.snapshot["decisions"]
                                      if d["process"] != process["id"]]
        return {"type": "process", "process": deepcopy(process)}

    def dispatch(self, request):
        kind = request["type"]
        if kind == "snapshot":
            return self.current()
        if kind == "hello":
            return {"type": "hello", "version": self.protocol_version}
        if kind == "logs":
            process = self.process(request["name"])
            return {"type": "logs", "text": self.logs.get(process["name"], "") if process else ""}
        if kind in ("recognize", "unrecognize", "stop_discovered"):
            process = self.process(request["process"]["id"])
            if not process or process["origin"] != "unmanaged" or process["status"]["state"] == "exited":
                return {"type": "error", "message": "Fixture process is no longer running."}
            if kind == "stop_discovered":
                return self.stop(process)
            app = self.app(process)
            if kind == "recognize":
                if app and not any(a["path"] == app for a in self.snapshot["recognized"]):
                    self.snapshot["recognized"].append({"path": app, "recognized_at_ms": self.epoch_ms})
                elif not app:
                    self.process_rules.add(process["id"])
            else:
                self.process_rules.discard(process["id"])
                self.snapshot["recognized"] = [a for a in self.snapshot["recognized"] if a["path"] != app]
        elif kind == "forget":
            self.snapshot["recognized"] = [a for a in self.snapshot["recognized"] if a["path"] != request["app"]]
        elif kind == "answer":
            decision = next((d for d in self.snapshot["decisions"] if d["id"] == request["decision"]), None)
            if not decision:
                return {"type": "error", "message": "Fixture decision is already resolved."}
            process = self.process(decision["process"])
            if request["answer"] == "kill":
                self.stop(process)
            else:
                process["status"] = {"state": "running"}
                self.snapshot["decisions"].remove(decision)
        elif kind in ("stop", "restart", "clear"):
            process = self.process(request["name"])
            if not process or process["origin"] != "managed":
                return {"type": "error", "message": "No matching managed fixture process."}
            if kind == "stop":
                return self.stop(process)
            if kind == "clear":
                self.snapshot["processes"].remove(process)
            else:
                process.update(status={"state": "running"}, started_at_ms=self.epoch_ms,
                               metrics={"cpu_percent": 0, "rss_bytes": 80 * 1024**2})
                self.snapshot["decisions"] = [d for d in self.snapshot["decisions"]
                                              if d["process"] != process["id"]]
                return {"type": "process", "process": deepcopy(process)}
        elif kind == "set_automatic_update_checks":
            self.snapshot["updates"]["automatic_checks"] = request["enabled"]
        elif kind not in ("shutdown", "check_for_updates"):
            return {"type": "error", "message": f"Fixture engine does not implement {kind}."}
        return {"type": "ok"}


class FixtureServer(socketserver.UnixStreamServer):
    def __init__(self, path, fixture, verbose=False):
        self.fixture = fixture
        self.verbose = verbose
        super().__init__(str(path), FixtureHandler)


class FixtureHandler(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(2)
        try:
            request = json.loads(self.rfile.readline(65536))
            if self.server.verbose:
                print(f"Fixture request: {request.get('type', 'invalid')}", flush=True)
            response = self.server.fixture.dispatch(request)
        except (ValueError, KeyError, TypeError) as error:
            response = {"type": "error", "message": f"Invalid fixture request: {error}"}
        self.wfile.write((json.dumps(response) + "\n").encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=SCENARIOS, default="process-list")
    parser.add_argument("--app", type=Path, default=Path.home() / "Applications/Pinch.app")
    parser.add_argument("--serve-only", action="store_true", help="Print the isolated socket directory without launching the UI")
    parser.add_argument("--verbose", action="store_true", help="Print received request types")
    args = parser.parse_args()
    binary = args.app.expanduser().resolve() / "Contents/MacOS/pinch-ui"
    if not args.serve_only and not binary.is_file():
        parser.error(f"No bundled UI at {binary}; build/install Pinch or pass --app dist/Pinch.app")
    with tempfile.TemporaryDirectory(prefix="pinch-fixture-") as directory:
        home = Path(directory)
        (home / "onboarded").touch()
        with FixtureServer(home / "pinchd.sock", Fixture(args.scenario), args.verbose) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            ui = None
            print(f"Fixture: {args.scenario}\nPINCH_HOME={directory}\nCtrl-C closes this fixture session.", flush=True)
            try:
                if args.serve_only:
                    thread.join()
                else:
                    ui = subprocess.Popen([str(binary), "--dev"], env=dict(os.environ, PINCH_HOME=directory), start_new_session=True)
                    ui.wait()
            except KeyboardInterrupt:
                pass
            finally:
                if ui is not None and ui.poll() is None:
                    ui.terminate()
                    try:
                        ui.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        ui.kill()
                        ui.wait(timeout=5)
                server.shutdown()
                thread.join()


if __name__ == "__main__":
    main()

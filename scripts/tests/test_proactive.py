"""Exercise proactive decisions against an isolated real engine.

Run after cargo build --workspace: python3 -m unittest scripts.tests.test_proactive -v
"""

import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ProactiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pinch-proactive-")
        self.home = Path(self.temp.name)
        self.env = dict(os.environ, PINCH_HOME=str(self.home), PINCH_STALE_AFTER_SECS="4")
        self.names = []
        self.log = (self.home / "engine.log").open("a")
        self.start_engine()

    def start_engine(self):
        self.engine = subprocess.Popen(
            [str(ROOT / "target/debug/pinchd"), "--headless"],
            env=self.env, stdout=self.log, stderr=self.log,
        )
        self.wait_for(lambda: (self.home / "pinchd.sock").exists())

    def stop_engine(self):
        self.rpc({"type": "shutdown"})
        self.engine.wait(timeout=5)

    def tearDown(self):
        try:
            for name in self.names:
                self.rpc({"type": "stop", "name": name})
            self.stop_engine()
        finally:
            if self.engine.poll() is None:
                self.engine.terminate()
                self.engine.wait(timeout=5)
            self.log.close()
            self.temp.cleanup()

    def rpc(self, request):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(10)
            connection.connect(str(self.home / "pinchd.sock"))
            connection.sendall((json.dumps(request) + "\n").encode())
            with connection.makefile() as stream:
                response = json.loads(stream.readline())
        self.assertNotEqual(response["type"], "error", response)
        return response

    def snapshot(self):
        return self.rpc({"type": "snapshot"})

    def ensure(self, name, code, agent=None):
        if name not in self.names:
            self.names.append(name)
        return self.rpc({"type": "ensure", "name": name,
                         "command": [sys.executable, "-u", "-c", code],
                         "cwd": str(self.home), "owner": "proactive-test",
                         "agent": agent or {"harness": "codex", "thread_id": "proactive-test", "agent_id": "worker-1"}})

    def answer(self, decision, answer):
        return self.rpc({"type": "answer", "decision": decision["id"], "answer": answer})

    def wait_for(self, predicate, timeout=16):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            time.sleep(0.15)
        self.fail("Condition timed out")

    def assert_quiet_for(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.assertEqual(self.snapshot()["decisions"], [])
            time.sleep(0.2)

    def test_decision_survives_restart_ensure_resets_and_keep_is_persistent(self):
        code = "import time; time.sleep(180)"
        process = self.ensure("idle", code)["process"]
        attribution = process["agent"]
        self.assertEqual(attribution["harness"], "codex")
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.assertGreaterEqual(decision["quiet_since_ms"], process["started_at_ms"])
        self.assertLess(decision["quiet_since_ms"], decision["created_at_ms"])
        self.assertIn("MB RAM", decision["reason"])
        self.assertIn("could be freed", decision["reason"])
        self.assertEqual(self.snapshot()["notifications"], "unavailable")
        self.stop_engine()
        self.start_engine()
        restored = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.assertEqual(decision["id"], restored["id"])
        restored_process = next(p for p in self.snapshot()["processes"] if p["id"] == process["id"])
        self.assertEqual(restored_process["agent"], attribution)
        reused = self.ensure("idle", code, {"harness": "claude-code", "thread_id": "another-session"})
        self.assertEqual(reused["process"]["pid"], process["pid"])
        self.assertEqual(reused["process"]["agent"], attribution)
        self.assert_quiet_for(2)
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.answer(decision, "keep")
        self.stop_engine()
        self.start_engine()
        self.assert_quiet_for(7)
        restarted = self.rpc({"type": "restart", "name": "idle"})["process"]
        self.assertNotEqual(restarted["pid"], process["pid"])
        self.assertEqual(restarted["agent"], attribution)
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.assertEqual(decision["process"], restarted["id"])

    def test_legacy_state_restores_without_delaying_cleanup_and_stop_terminates_process(self):
        original = self.ensure("idle", "import time; time.sleep(180)")["process"]
        self.stop_engine()
        state = json.loads((self.home / "state.json").read_text())
        # Older installations persisted a suppression deadline. It no longer applies.
        state[0]["snoozed_until_ms"] = int(time.time() * 1000) + 3_600_000
        (self.home / "state.json").write_text(json.dumps(state))
        self.start_engine()
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        restored = self.snapshot()["processes"][0]
        self.assertEqual(restored["pid"], original["pid"])
        self.answer(decision, "kill")
        process = self.snapshot()["processes"][0]
        self.assertEqual(process["status"]["state"], "exited")
        self.assertIsNone(process["metrics"])
        self.assertEqual(self.snapshot()["decisions"], [])

    def test_observed_connection_postpones_suggestion_after_disconnect(self):
        self.ensure("server", "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); "
                    "s.listen(); c,_=s.accept(); c.recv(1); c.close(); "
                    "import time; time.sleep(180)")
        def ports():
            return next((p["ports"] for p in self.snapshot()["processes"]
                         if p["name"] == "server" and p["ports"]), None)
        port = self.wait_for(ports)[0]
        with socket.create_connection(("127.0.0.1", port)):
            self.assert_quiet_for(7)
        self.assert_quiet_for(1.5)
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.assertEqual(decision["process_name"], "server")

    def test_cpu_burst_does_not_keep_an_idle_process_busy_forever(self):
        self.ensure("burst", "import time\nuntil=time.monotonic()+3\n"
                    "while time.monotonic()<until: pass\n"
                    "time.sleep(180)")
        decision = self.wait_for(lambda: self.snapshot()["decisions"])[0]
        self.assertEqual(decision["process_name"], "burst")
        process = next(p for p in self.snapshot()["processes"] if p["name"] == "burst")
        self.assertLess(process["metrics"]["cpu_percent"], 5)

    def test_busy_cpu_and_log_output_prevent_suggestions(self):
        self.ensure("busy", "while True: pass")
        self.ensure("chatty", "import time\nwhile True:\n print('working'); time.sleep(.5)")
        self.assert_quiet_for(9)
        self.assertTrue(all(p["origin"] == "managed" for p in self.snapshot()["processes"]
                            if p["name"] in self.names))


if __name__ == "__main__":
    unittest.main()

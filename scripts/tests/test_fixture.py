import importlib.util
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("fixture", Path(__file__).parents[1] / "fixture.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.data = fixture.Fixture()

    def test_rebasing_preserves_ages_without_mutating_source_data(self):
        original = json.loads(json.dumps(self.data.snapshot))
        with patch.object(fixture.time, "time", return_value=2_000_000_000):
            snapshot = self.data.current()
        for actual, source in zip(snapshot["processes"], original["processes"]):
            self.assertEqual(2_000_000_000_000 - actual["started_at_ms"],
                             self.data.epoch_ms - source["started_at_ms"])
        self.assertEqual(self.data.snapshot, original)

    def test_recognition_and_undo_change_the_returned_snapshot(self):
        process = self.data.current()["processes"][2]
        self.data.dispatch({"type": "recognize", "process": process})
        self.assertTrue(self.data.current()["processes"][2]["recognized"])
        self.data.dispatch({"type": "unrecognize", "process": process})
        self.assertFalse(self.data.current()["processes"][2]["recognized"])
        self.data.dispatch({"type": "forget", "app": "/Applications/Raycast.app"})
        self.assertFalse(self.data.current()["processes"][3]["recognized"])
        self.assertEqual(self.data.current()["recognized"], [])

    def test_stop_and_stale_answers_only_mutate_fixture_records(self):
        decision = self.data.current()["decisions"][0]
        self.data.dispatch({"type": "answer", "decision": decision["id"], "answer": "kill"})
        self.assertEqual(self.data.current()["decisions"], [])
        self.assertEqual(self.data.current()["processes"][0]["status"]["state"], "exited")
        external = self.data.current()["processes"][3]
        self.data.dispatch({"type": "stop_discovered", "process": external})
        self.assertEqual(self.data.current()["processes"][3]["status"]["state"], "exited")
        self.assertEqual(fixture.Fixture().current()["processes"][3]["status"]["state"], "running")

    def test_keep_returns_stale_process_to_managed_and_logs_are_available(self):
        self.data.dispatch({"type": "answer", "decision": "fixture-stale-docs", "answer": "keep"})
        snapshot = self.data.current()
        self.assertEqual(snapshot["decisions"], [])
        self.assertEqual(snapshot["processes"][0]["status"]["state"], "running")
        self.assertIn("http://localhost:3001", self.data.dispatch({"type": "logs", "name": "docs"})["text"])

    def test_unknown_commands_cannot_launch_real_processes(self):
        self.assertEqual(self.data.dispatch({"type": "run"})["type"], "error")
        self.assertEqual(self.data.dispatch({"type": "ensure"})["type"], "error")

    def test_scenarios_cover_empty_notifications_and_updates(self):
        self.assertEqual(fixture.Fixture("empty").current()["processes"], [])
        self.assertEqual(fixture.Fixture("notifications-disabled").current()["notifications"], "disabled")
        updates = fixture.Fixture("updates-available")
        self.assertEqual(updates.current()["updates"]["available_version"], "0.3.0")
        updates.dispatch({"type": "set_automatic_update_checks", "enabled": False})
        self.assertFalse(updates.current()["updates"]["automatic_checks"])

    def test_server_speaks_the_same_newline_json_socket_protocol(self):
        with tempfile.TemporaryDirectory(prefix="pinch-fixture-test-") as directory:
            path = Path(directory) / "pinchd.sock"
            with fixture.FixtureServer(path, self.data) as server:
                thread = threading.Thread(target=server.serve_forever)
                thread.start()
                try:
                    with socket.socket(socket.AF_UNIX) as connection:
                        connection.settimeout(2)
                        connection.connect(str(path))
                        connection.sendall(b'{"type":"snapshot"}\n')
                        response = json.loads(connection.makefile().readline())
                    self.assertEqual(response["type"], "snapshot")
                    self.assertEqual(response["processes"][0]["id"], self.data.snapshot["processes"][0]["id"])
                finally:
                    server.shutdown()
                    thread.join()


if __name__ == "__main__":
    unittest.main()

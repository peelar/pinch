import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("dev", Path(__file__).parents[1] / "dev.py")
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        (root / "target/debug").mkdir(parents=True)
        (root / "preview").mkdir()
        self.addCleanup(patch.stopall)
        patch.object(dev, "ROOT", root).start()
        patch.object(dev, "DEV", root / "preview").start()
        self.build = patch.object(dev.subprocess, "run", return_value=Mock(returncode=0)).start()
        self.spawn = patch.object(
            dev.subprocess, "Popen", side_effect=lambda *a, **kw: Mock(poll=Mock(return_value=None))
        ).start()
        self.stop = patch.object(dev, "stop").start()
        patch.object(dev, "wait_for_engine").start()
        for name in ("pinchd", "pinch-ui", "pinch"):
            (root / "target/debug" / name).write_text(name)
        self.preview = dev.Preview()
        self.preview.rebuild()
        self.stop.reset_mock()
        self.spawn.reset_mock()

    def change(self, name):
        (dev.ROOT / "target/debug" / name).write_text(name + " changed")

    def test_ui_edit_keeps_engine(self):
        engine, ui = self.preview.engine, self.preview.ui
        self.change("pinch-ui")
        self.preview.rebuild()
        self.assertIs(self.preview.engine, engine)
        self.stop.assert_called_once_with(ui)
        self.assertEqual(self.spawn.call_count, 1)

    def test_engine_edit_restarts_both(self):
        engine, ui = self.preview.engine, self.preview.ui
        self.change("pinchd")
        self.preview.rebuild()
        self.assertEqual([call.args[0] for call in self.stop.call_args_list], [ui, engine])
        self.assertEqual(self.spawn.call_count, 2)

    def test_failed_build_retains_previous_binaries_and_processes(self):
        self.change("pinch-ui")
        self.build.return_value.returncode = 1
        self.preview.rebuild()
        self.stop.assert_not_called()
        self.spawn.assert_not_called()
        self.assertEqual((dev.DEV / "pinch-ui").read_text(), "pinch-ui")

    def test_unchanged_build_does_not_restart(self):
        self.preview.rebuild()
        self.stop.assert_not_called()
        self.spawn.assert_not_called()

    def test_closed_ui_reopens_on_next_successful_build(self):
        self.preview.ui.poll.return_value = 0
        engine = self.preview.engine
        self.preview.rebuild()
        self.assertIs(self.preview.engine, engine)
        self.assertEqual(self.spawn.call_count, 1)


if __name__ == "__main__":
    unittest.main()

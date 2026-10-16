"""Regression checks for the boundary guard's rejection paths."""

import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "architecture", Path(__file__).resolve().parents[1] / "check_architecture.py")
architecture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(architecture)


class ArchitectureTests(unittest.TestCase):
    def test_rejects_desktop_dependencies_even_when_transitive(self):
        errors = architecture.check_dependencies(
            "pinch-engine v0.1.0\nhelper v0.1.0\nobjc2-app-kit v0.3.2\ngpui_platform v0.1.0\n")
        self.assertEqual(len(errors), 2)
        self.assertEqual(architecture.check_dependencies(
            "sysinfo v0.39.6\nobjc2-foundation v0.3.2\nobjc2-io-kit v0.3.2\n"), [])

    def test_rejects_feature_gated_native_code_and_misplaced_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "crates/engine/src"
            (source / "desktop").mkdir(parents=True)
            (source / "discovery").mkdir()
            (source / "desktop/mod.rs").write_text("use pinch_macos::Handle;")
            (source / "discovery/lsof.rs").write_text('Command::new("/usr/sbin/lsof");')
            self.assertEqual(architecture.check_sources(root), [])
            client = root / "crates/client/src"
            client.mkdir(parents=True)
            (client / "lib.rs").write_text("use gpui::App;")
            (source / "server.rs").write_text(
                '#[cfg(feature = "desktop")]\nuse pinch_macos::Handle;\n')
            (source / "engine.rs").write_text('Command::new("/usr/sbin/lsof");')
            errors = architecture.check_sources(root)
            self.assertEqual(len(errors), 3)
            self.assertTrue(any("server.rs:2" in error for error in errors))
            self.assertTrue(any("engine.rs:1" in error for error in errors))

"""Installer boundary checks; fixture binaries never contact a real engine."""

import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="pinch-install-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.apps = self.root / "apps"
        self.bin = self.root / "bin"
        self.apps.mkdir()
        self.bin.mkdir()
        self.quit_log = self.root / "quit.log"
        self.env = dict(os.environ, PINCH_APP_DIR=str(self.apps), PINCH_BIN_DIR=str(self.bin),
                        PINCH_HOME="/tmp/do-not-stop-this-engine", PINCH_TEST_QUIT_LOG=str(self.quit_log))
        self.bundle = self.make_bundle(self.root / "build/Pinch Proactive.app", "proactive")
        self.stable = self.make_bundle(self.apps / "Pinch.app", None)
        (self.stable / "preserve-me").write_text("stable")
        (self.bin / "pinch").symlink_to(self.stable / "Contents/MacOS/pinch")

    def make_bundle(self, path, variant):
        macos = path / "Contents/MacOS"
        macos.mkdir(parents=True)
        for name in ("pinch", "pinchd", "pinch-ui"):
            binary = macos / name
            binary.write_text('#!/bin/bash\nif [[ "$*" == *quit* ]]; then printf "%s\\n" "${PINCH_HOME-unset}" >> "$PINCH_TEST_QUIT_LOG"; fi\n')
            binary.chmod(0o755)
        bundle_id = "sh.pinch.Pinch" + (f".{variant}" if variant else "")
        with (path / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump({"CFBundleIdentifier": bundle_id}, stream)
        if variant:
            resources = path / "Contents/Resources"
            resources.mkdir()
            (resources / "pinch-variant").write_text(variant + "\n")
        return path

    def install(self, *extra):
        return subprocess.run([str(ROOT / "scripts/install.sh"), "--app", str(self.bundle),
                               "--no-modify-path", *extra], env=self.env, text=True, capture_output=True)

    def test_variant_install_and_update_leave_main_installation_alone(self):
        stable_link = os.readlink(self.bin / "pinch")
        for _ in range(2):
            result = self.install()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((self.stable / "preserve-me").read_text(), "stable")
            self.assertEqual(os.readlink(self.bin / "pinch"), stable_link)
        self.assertEqual(self.quit_log.read_text().splitlines(), ["unset", "unset"])
        self.assertEqual(os.readlink(self.bin / "pinch-proactive"),
                         str(self.apps / "Pinch Proactive.app/Contents/MacOS/pinch"))

    def test_mismatched_requested_variant_fails_before_quitting(self):
        result = self.install("--variant", "other")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.quit_log.exists())
        self.assertFalse((self.apps / "Pinch Proactive.app").exists())

    def test_existing_app_with_another_identity_is_not_replaced(self):
        existing = self.make_bundle(self.apps / "Pinch Proactive.app", "other")
        (existing / "preserve-me").write_text("unrelated")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.quit_log.exists())
        self.assertEqual((existing / "preserve-me").read_text(), "unrelated")

    def test_invalid_variant_cannot_escape_the_destination(self):
        (self.bundle / "Contents/Resources/pinch-variant").write_text("../main")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.quit_log.exists())
        self.assertEqual((self.stable / "preserve-me").read_text(), "stable")

    def test_bundle_identifier_must_match_variant_metadata(self):
        with (self.bundle / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump({"CFBundleIdentifier": "sh.pinch.Pinch"}, stream)
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.quit_log.exists())


if __name__ == "__main__":
    unittest.main()

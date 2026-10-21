"""The stable feed must pin a signed archive to its own release, never 'latest'."""
import base64
import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("appcast", Path(__file__).parents[1] / "appcast.py")
appcast = importlib.util.module_from_spec(spec)
spec.loader.exec_module(appcast)


class AppcastTests(unittest.TestCase):
    def test_archive_version_signature_length_and_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "pinch-macos-arm64.zip"
            archive.write_bytes(b"archive")
            signature = base64.b64encode(bytes(64)).decode()
            xml = appcast.appcast("peelar/pinch", "1.2.3", archive, signature, "Fix <restart> & preserve servers")
            item = ET.fromstring(xml).find("channel/item")
            enclosure = item.find("enclosure")
            self.assertEqual(enclosure.attrib["url"], "https://github.com/peelar/pinch/releases/download/v1.2.3/pinch-macos-arm64.zip")
            self.assertEqual(enclosure.attrib["length"], "7")
            self.assertEqual(enclosure.attrib[f"{{{appcast.SPARKLE}}}edSignature"], signature)
            self.assertEqual(item.find("description").text, "Fix <restart> & preserve servers")
            for version in ("1.2.3-beta.1", "latest", "../escape"):
                with self.assertRaises(ValueError):
                    appcast.appcast("peelar/pinch", version, archive, signature, "")
            with self.assertRaises(ValueError):
                appcast.appcast("peelar/pinch", "1.2.3", archive, "invalid", "")

    def test_notes_include_only_the_matching_version(self):
        with tempfile.TemporaryDirectory() as directory:
            changelog = Path(directory) / "CHANGELOG.md"
            changelog.write_text("# pinch\n\n## 1.2.3\n\nNew notes\n\n## 1.2.2\n\nOld notes\n")
            self.assertEqual(appcast.release_notes(changelog, "1.2.3"), "New notes")
            with self.assertRaises(ValueError):
                appcast.release_notes(changelog, "9.0.0")

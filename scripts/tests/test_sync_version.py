import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class SyncVersionTests(unittest.TestCase):
    def test_version_bump_updates_lock_and_check_rejects_stale_lock(self):
        script = Path(__file__).resolve().parents[1] / 'sync-version.mjs'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            shutil.copy(script, root / 'scripts/sync-version.mjs')
            (root / 'src').mkdir()
            (root / 'src/lib.rs').write_text('')
            (root / 'Cargo.toml').write_text(
                '[workspace]\nmembers = ["."]\n'
                '[workspace.package]\nversion = "0.1.0"\n'
                '[package]\nname = "pinch-fixture"\nversion.workspace = true\n'
            )
            (root / 'package.json').write_text(json.dumps({'version': '0.1.0'}))
            subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=root, check=True)
            stale_lock = (root / 'Cargo.lock').read_text()
            (root / 'package.json').write_text(json.dumps({'version': '0.2.0'}))
            command = ['node', 'scripts/sync-version.mjs']
            subprocess.run(command, cwd=root, check=True)
            self.assertIn('version = "0.2.0"', (root / 'Cargo.lock').read_text())
            subprocess.run(command + ['--check'], cwd=root, check=True)
            (root / 'Cargo.lock').write_text(stale_lock)
            result = subprocess.run(command + ['--check'], cwd=root, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / 'Cargo.lock').read_text(), stale_lock)

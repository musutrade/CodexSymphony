import contextlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import install_evidence_archive as installer


class RetentionInstall(unittest.TestCase):
    def source(self, root):
        (root/'tools').mkdir()
        for name in installer.NAMES:
            (root/'tools'/name).write_text('# fixture '+name+'\n')

    def test_immutable_release_and_units_use_same_fixed_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.source(root)
            base = root/'host'
            release = installer.install_release(root, base)
            self.assertEqual(installer.install_release(root, base), release)
            units = root/'units'
            installer.install_units(base, release, units)
            for name in ['codexsymphony-archive.service', 'codexsymphony-cache-retention.service',
                         'codexsymphony-capture-cache.service']:
                content = (units/name).read_text()
                self.assertIn(str(release), content)
                self.assertIn('ConditionPathIsMountPoint='+str(base), content)
            self.assertIn('OnUnitInactiveSec=1h', (units/'codexsymphony-cache-retention.timer').read_text())
            (release/'cache_retention.py').write_text('tampered')
            with self.assertRaisesRegex(ValueError, 'immutable'):
                installer.install_release(root, base)
            self.assertEqual((release/'cache_retention.py').read_text(), 'tampered')

    def test_install_registers_without_starting_unvalidated_cleanup(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.source(root)
            with patch.object(installer, '__file__', str(root/'tools/install_evidence_archive.py')), \
                 patch.object(Path, 'home', return_value=root), \
                 patch.object(installer.subprocess, 'run') as run, \
                 contextlib.redirect_stdout(io.StringIO()):
                installer.main()
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(len(commands), 6)
            self.assertEqual(commands[0], ['systemctl', '--user', 'daemon-reload'])
            self.assertTrue(all(command[2] == 'enable' for command in commands[1:]))
            self.assertTrue(all('--now' not in command and 'start' not in command for command in commands))


if __name__ == '__main__':
    unittest.main()

class RetentionDeployment(unittest.TestCase):
    def test_manifest_covers_releases_units_and_persistent_guards(self):
        import json
        import hashlib
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'tools').mkdir()
            for name in installer.NAMES:
                (root/'tools'/name).write_text('fixture')
            release = installer.install_release(root, root/'state')
            units = root/'units'
            installer.install_units(root/'state', release, units)
            installer.install_guard(root/'state', release, units)
            receipt = json.loads((root/'state/storage-maintenance/deployment.json').read_text())
            self.assertEqual(set(receipt['commands']), set(installer.SERVICES))
            self.assertIn('--guard-retention', (units/'codexsymphony-retention-watch.service').read_text())
            self.assertFalse((units/'codexsymphony-retention-watch.service.d').exists())
            self.assertFalse((units/'codexsymphony-storage.service.d').exists())
            for name, digest in receipt['files'].items():
                self.assertEqual(hashlib.sha256(Path(name).read_bytes()).hexdigest(), digest)
            for name in installer.GUARDED:
                guard = units/(name+'.d')/'retention-guard.conf'
                self.assertIn('--check-retention', guard.read_text())
                self.assertIn(str(guard), receipt['files'])

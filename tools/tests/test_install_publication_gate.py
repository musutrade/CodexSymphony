import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('install_publication_gate', ROOT / 'tools/install_publication_gate.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class LeaseBindingTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.release = self.base / 'release'
        self.release.mkdir()
        self.volume = self.base / 'volume'
        (self.release / 'fixed_workspace.py').write_text('MARK = 1\n')
        (self.release / 'bounded_layout.py').write_text(
            'from pathlib import Path\nimport fixed_workspace\n'
            'VOLUME = Path(' + repr(str(self.volume)) + ')\n'
            'def slot():\n    return VOLUME / "validation/gate"\n')
        for lease in self.leases():
            lease.parent.mkdir(parents=True)
            lease.touch()

    def leases(self):
        return [self.volume / 'cache/coordination/writer.lock', self.volume / 'validation/gate/writer.lock']

    def approval(self, **changes):
        pins = {str(self.release / name): hashlib.sha256((self.release / name).read_bytes()).hexdigest()
                for name in ('fixed_workspace.py', 'bounded_layout.py')}
        return {'execution_version': 3, 'host_release': str(self.release), 'runtime_files': pins} | changes

    def test_v3_binding_resolves_leases_from_the_approved_host_layout(self):
        approval = self.approval()
        binding = installer.lease_binding(approval)
        self.assertEqual(binding, {'approval': installer.contract.digest(approval), 'host_release': str(self.release),
                                   'leases': [str(p) for p in self.leases()]})

    def test_changed_or_unpinned_host_module_is_refused(self):
        approval = self.approval()
        (self.release / 'bounded_layout.py').write_text('VOLUME = None\n')
        with self.assertRaisesRegex(ValueError, 'changed: bounded_layout'):
            installer.lease_binding(approval)
        approval = self.approval()
        del approval['runtime_files'][str(self.release / 'fixed_workspace.py')]
        with self.assertRaisesRegex(ValueError, 'changed: fixed_workspace'):
            installer.lease_binding(approval)

    def test_missing_or_aliased_lease_is_refused(self):
        self.leases()[1].unlink()
        with self.assertRaisesRegex(ValueError, 'lease missing'):
            installer.lease_binding(self.approval())
        self.leases()[1].symlink_to(self.leases()[0])
        with self.assertRaisesRegex(ValueError, 'aliased'):
            installer.lease_binding(self.approval())

    def test_non_v3_approval_binds_no_leases(self):
        approval = self.approval(execution_version=2)
        self.assertEqual(installer.lease_binding(approval)['leases'], [])

    def test_install_writes_the_binding_into_the_content_addressed_release(self):
        home = self.base / 'home'
        approval_path = self.base / 'approval.json'
        approval_path.write_text(json.dumps(self.approval()))
        with patch.object(installer, 'HOME', home), patch.object(installer, 'APPROVAL', approval_path), \
                patch('builtins.print') as printed:
            installer.main()
        result = json.loads(printed.call_args.args[0])
        release = Path(result['release'])
        bound = json.loads((release / 'host-leases.json').read_text())
        self.assertEqual(bound['leases'], [str(p) for p in self.leases()])
        self.assertIn(str(release / 'host-leases.json'), json.loads((release / 'installed-files.json').read_text()))
        self.assertIn(str(release / 'gate.py'), (home / 'guard').read_text())
        (release / 'host-leases.json').write_text('{}')
        with patch.object(installer, 'HOME', home), patch.object(installer, 'APPROVAL', approval_path), \
                patch('builtins.print'), self.assertRaisesRegex(ValueError, 'release differs'):
            installer.main()


if __name__ == '__main__':
    unittest.main()

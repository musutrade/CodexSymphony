import importlib.util
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('sandbox',Path(__file__).with_name('trusted_environment.py'))
sandbox=importlib.util.module_from_spec(spec)
spec.loader.exec_module(sandbox)


class ResolverMountTests(unittest.TestCase):
    def test_systemd_symlink_mounts_only_resolved_file_readonly(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            target=root/'run/systemd/resolve/stub-resolv.conf'
            target.parent.mkdir(parents=True)
            target.write_text('nameserver 127.0.0.53\n')
            link=root/'resolv.conf'
            link.symlink_to(target)
            self.assertEqual(sandbox.resolver_mount(link),['--ro-bind',str(target),str(target)])

    def test_regular_resolver_and_missing_target(self):
        with tempfile.TemporaryDirectory() as directory:
            resolver=Path(directory)/'resolv.conf'
            resolver.write_text('nameserver 127.0.0.53\n')
            self.assertEqual(sandbox.resolver_mount(resolver),['--ro-bind',str(resolver),str(resolver)])
            resolver.unlink()
            with self.assertRaises(FileNotFoundError):
                sandbox.resolver_mount(resolver)


if __name__=='__main__':
    unittest.main()

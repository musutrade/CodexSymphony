from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import isolation as iso


class IsolationTargetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.run = self.base / 'run'; self.run.mkdir()
        self.codex = self.base / 'bin'; self.codex.mkdir(); (self.codex / 'codex').write_text('fixture')
        self.policy = {'test_environment': {'SCCACHE_DIR': '/fixed/sccache'}}
        for name, value in [('load', self.policy), ('codex_bin', self.codex), ('test_environment', {'SCCACHE_DIR': '/fixed/sccache'})]:
            p = patch.object(iso.contract, name, return_value=value); p.start(); self.addCleanup(p.stop)

    def args(self, **kwargs):
        return iso.command(['true'], run=self.run, repository=self.base, plugins=self.base, **kwargs)

    def test_explicit_target_is_reused_and_all_requested_mounts_remain(self):
        target = self.base / 'cache'; target.mkdir()
        argv = self.args(compiler_target=target, readonly=[self.base / 'read'], writable=[target],
                         mounts=[(self.base / 'source', self.base / 'destination')], environment={'EXAMPLE': 'value'}, cwd=self.base)
        index = argv.index('CARGO_TARGET_DIR')
        self.assertEqual(argv[index + 1], str(target))
        self.assertTrue((target / 'sccache').is_dir())
        self.assertIn(str(self.base / 'read'), argv)
        self.assertIn(str(self.base / 'destination'), argv)
        self.assertIn('EXAMPLE', argv)
        self.assertIn('value', argv)
        self.assertFalse((self.run / 'target').exists())

    def test_default_target_and_missing_pinned_runtime(self):
        argv = self.args()
        self.assertIn(str(self.run / 'target'), argv)
        (self.codex / 'codex').unlink()
        with self.assertRaisesRegex(ValueError, 'runtime mount missing'):
            self.args()

    def test_namespace_codex_fallback(self):
        with patch.object(iso.contract, 'codex_bin', return_value=self.base / 'missing'), patch.object(Path, 'is_file', return_value=True):
            argv = self.args()
        self.assertIn('/opt/codex', argv)


if __name__ == '__main__': unittest.main()

import json
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import cache_retention as c


class CacheRetention(unittest.TestCase):
    def fixture(self, root):
        cache = root / 'target/debug'
        cache.mkdir(parents=True)
        (cache / 'object').write_bytes(b'rebuildable')
        evidence = root / 'target/llvm-cov-target/raw'
        evidence.mkdir(parents=True)
        (evidence / 'counter').write_text('retain')
        os.utime(cache / 'object', (0, 0))
        os.utime(cache, (0, 0))
        return cache, evidence

    def test_dry_run_and_apply_keep_coverage_and_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache, evidence = self.fixture(root)
            release = root / 'target/release'
            release.mkdir()
            (release / 'server').write_text('running binary')
            args = {'budget': 1, 'now': 100000, 'busy': lambda _: False}
            self.assertEqual(c.collect(cache, **args)['status'], 'WOULD_CLEAN')
            self.assertTrue(cache.exists())
            result = c.collect(cache, apply=True, **args)
            self.assertEqual(result['status'], 'CLEANED')
            self.assertGreater(result['bytes_before'], 1)
            self.assertEqual((evidence / 'counter').read_text(), 'retain')
            self.assertEqual((release / 'server').read_text(), 'running binary')

    def test_budget_recent_and_active_caches_are_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            cache, _ = self.fixture(Path(tmp))
            self.assertEqual(c.collect(cache, c.GIB, now=100000)['status'], 'WITHIN_BUDGET')
            self.assertEqual(c.collect(cache, 1, now=1)['status'], 'DEFERRED_RECENT')
            self.assertEqual(c.collect(cache, 1, now=100000, busy=lambda _: True)['status'], 'DEFERRED_BUSY')
            with patch.object(c, 'in_use', side_effect=[False, True]) as busy:
                self.assertEqual(c.collect(cache, 1, apply=True, now=100000, busy=busy)['status'], 'DEFERRED_BUSY')
            self.assertTrue(cache.exists())

    def test_symlink_and_mount_cannot_delete_external_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache, evidence = self.fixture(root)
            link = root / 'link'
            link.symlink_to(cache)
            with self.assertRaises(ValueError):
                c.collect(link, 1, apply=True)
            with patch.object(Path, 'is_mount', lambda p: p == cache):
                with self.assertRaises(ValueError):
                    c.collect(cache, 1, apply=True)
            self.assertTrue((evidence / 'counter').exists())

    def test_missing_cache_and_ssd_alias_busy_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(c.collect(Path(tmp) / 'absent', 1)['status'], 'ABSENT')
        shared = Path.home() / 'cargo-target/debug'
        self.assertIn(Path('/mnt/dev-ssd/cargo-target/debug'), c.aliases(shared))
        historical = c.ROOT / 'gh88-product-acceptance/build/target/debug'
        self.assertIn(Path('/mnt/dev-ssd/codexsymphony-state/gh88-product-acceptance/build/target/debug'), c.aliases(historical))
        with patch.object(c, 'gate_busy', side_effect=[False, True]):
            self.assertTrue(c.in_use(shared))
        with patch.object(c, 'gate_busy', return_value=False):
            self.assertFalse(c.in_use(shared))

    def test_failure_is_recorded_and_other_cache_still_processed(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp)
            with patch.object(c, 'policies', return_value=[(state/'bad', 1), (state/'ok', 1)]), \
                 patch.object(c, 'collect', side_effect=[ValueError('unsafe'), {'status': 'ABSENT'}]):
                result = c.maintain(state, apply=True)
            self.assertEqual([r['status'] for r in result['results']], ['ERROR', 'ABSENT'])
            self.assertEqual(json.loads((state/'cache-retention.json').read_text()), result)

    def test_policy_only_names_debug_and_unknown_alias_stays_local(self):
        self.assertEqual(len(c.policies()), 2)
        for path, budget in c.policies():
            self.assertEqual(path.name, 'debug')
            self.assertGreater(budget, 0)
        self.assertEqual(c.aliases(Path('/unrelated')), [Path('/unrelated')])

    def test_main_checks_mount_and_safe_deletion_support_before_maintenance(self):
        with patch('sys.argv', ['cache_retention']), patch.object(c.os.path, 'ismount', return_value=False):
            with self.assertRaisesRegex(ValueError, 'mount missing'):
                c.main()
        with patch('sys.argv', ['cache_retention']), patch.object(c.os.path, 'ismount', return_value=True), \
             patch.object(c.shutil.rmtree, 'avoids_symlink_attacks', False):
            with self.assertRaisesRegex(ValueError, 'fd-safe'):
                c.main()

    def test_main_returns_error_status_for_partial_failure_and_respects_apply(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'storage-maintenance').mkdir()
            for status in ['ERROR', 'ABSENT']:
                record = {'results': [{'status': status}]}
                with patch.object(c, 'ROOT', root), patch('sys.argv', ['cache_retention', '--apply']), \
                     patch.object(c.os.path, 'ismount', return_value=True), \
                     patch.object(c, 'maintain', return_value=record) as maintain, \
                     patch('sys.stdout', new_callable=io.StringIO):
                    if status == 'ERROR':
                        with self.assertRaises(SystemExit) as error:
                            c.main()
                        self.assertEqual(error.exception.code, 1)
                    else:
                        c.main()
                    maintain.assert_called_once_with(root/'storage-maintenance', apply=True)


if __name__ == '__main__':
    unittest.main()

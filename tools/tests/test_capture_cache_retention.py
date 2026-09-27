import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import capture_cache_retention as retention


class CaptureCacheRetention(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.state = self.base / 'state'
        self.state.mkdir()
        self.root = self.base / 'capture'
        raw = self.root / 'probes/backend/raw'
        raw.mkdir(parents=True)
        self.evidence = raw / 'profile.profraw'
        self.evidence.write_bytes(b'original counters')
        self.bundle = raw.parent / 'bundle.json'
        receipt = {'schema': 'rust-source-capture/v1', 'raw_root': str(raw),
                   'raw': {self.evidence.name: hashlib.sha256(self.evidence.read_bytes()).hexdigest()},
                   'context': {'commit': 'a' * 40}, 'inputs': {'Cargo.lock': 'b' * 64}}
        self.bundle.write_text(json.dumps({'request': {'parameters': {'receipt': receipt}}}))
        self.target = self.root / 'target'
        self.target.mkdir()
        (self.target / 'object').write_bytes(b'rebuildable')
        os.utime(self.target / 'object', (0, 0))
        os.utime(self.target, (0, 0))
        self.registry = self.state / 'capture-caches.json'
        self.entry = retention.register(self.registry, self.root)

    def test_dry_run_then_cleanup_retains_source_bundle_and_raw(self):
        with patch.object(retention, 'in_use', return_value=False):
            self.assertEqual(retention.collect_capture(self.entry)['status'], 'WOULD_CLEAN')
            self.assertTrue(self.target.exists())
            self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'CLEANED')
        self.assertEqual(self.evidence.read_bytes(), b'original counters')
        self.assertTrue(self.bundle.is_file())
        self.assertFalse(self.target.exists())

    def test_registered_completed_capture_needs_no_idle_delay(self):
        os.utime(self.target / 'object', None)
        os.utime(self.target, None)
        with patch.object(retention, 'in_use', return_value=False):
            self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'CLEANED')
        self.assertEqual(self.evidence.read_bytes(), b'original counters')

    def test_no_registration_never_discovers_or_removes_other_caches(self):
        self.registry.unlink()
        self.assertEqual(retention.maintain(self.state, apply=True)['results'], [])
        self.assertTrue(self.target.exists())

    def test_entire_capture_worker_and_late_worker_protect_target(self):
        with patch.object(retention, 'in_use', return_value=True) as busy:
            self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'DEFERRED_BUSY')
            busy.assert_called_with(self.root)
        with patch.object(retention, 'in_use', side_effect=[False, True]):
            self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'DEFERRED_BUSY')
        self.assertTrue(self.target.exists())

    def test_corrupt_raw_is_an_error_and_never_deletes_target(self):
        self.evidence.write_bytes(b'corrupt')
        with patch.object(retention, 'in_use', return_value=False):
            result = retention.maintain(self.state, apply=True)
        self.assertEqual(result['results'][0]['status'], 'ERROR')
        self.assertIn('checksum', result['results'][0]['error'])
        self.assertTrue(self.target.exists())

    def test_changed_registration_and_replaced_root_are_rejected(self):
        self.assertEqual(retention.register(self.registry, self.root), self.entry)
        self.bundle.write_text(self.bundle.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            retention.register(self.registry, self.root)
        with patch.object(retention, 'in_use', return_value=False):
            with self.assertRaisesRegex(ValueError, 'changed'):
                retention.collect_capture(self.entry, apply=True)
        self.root.rename(self.base / 'original')
        self.root.mkdir()
        with self.assertRaisesRegex(ValueError, 'replaced'):
            retention.collect_capture(self.entry, apply=True)

    def test_symlink_root_raw_and_target_are_not_followed(self):
        link = self.base / 'link'
        link.symlink_to(self.root)
        with self.assertRaisesRegex(ValueError, 'canonical'):
            retention.register(self.registry, link)
        self.target.rename(self.root / 'saved-target')
        self.target.symlink_to(self.root / 'saved-target')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            retention.collect_capture(self.entry, apply=True)
        self.assertTrue((self.root / 'saved-target/object').exists())

    def test_incomplete_capture_registration_is_rejected(self):
        value = json.loads(self.bundle.read_text())
        value['request']['parameters']['receipt']['raw'] = {}
        self.bundle.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'inventory'):
            retention.register(self.registry, self.root)

    def test_schema_missing_bundle_and_raw_identity_are_rejected(self):
        self.bundle.unlink()
        with self.assertRaisesRegex(ValueError, 'bundle'):
            retention.capture_receipt(self.root)
        for receipt in ({'schema': 'unknown'}, {'schema': 'rust-source-capture/v1', 'raw_root': '/wrong'}):
            with self.assertRaisesRegex(ValueError, 'identity'):
                retention.validate_receipt(receipt, self.evidence.parent)
        for field in ('raw', 'context', 'inputs'):
            receipt = {'schema': 'rust-source-capture/v1', 'raw_root': str(self.evidence.parent),
                       'raw': {'counter': 'hash'}, 'context': {'commit': 'a' * 40}, 'inputs': {'lock': 'hash'}}
            receipt[field] = {'commit': ''} if field == 'context' else {}
            with self.assertRaisesRegex(ValueError, 'inventory'):
                retention.validate_receipt(receipt, self.evidence.parent)

    def test_registry_invalid_schema_and_symlink_are_rejected(self):
        self.registry.write_text(json.dumps({'schema': 'unknown', 'captures': []}))
        with self.assertRaisesRegex(ValueError, 'registry'):
            retention.read_registry(self.registry)
        self.registry.unlink()
        self.registry.symlink_to(self.bundle)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            retention.read_registry(self.registry)

    def test_absent_cache_and_missing_capture_remain_safe(self):
        self.target.rename(self.root / 'saved-target')
        self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'ABSENT')
        self.root.rename(self.base / 'saved-capture')
        self.assertEqual(retention.collect_capture(self.entry, apply=True)['status'], 'ABSENT')

    def test_missing_raw_raw_symlink_and_traversal_are_rejected(self):
        raw = self.evidence.parent
        raw.rename(raw.with_name('saved-raw'))
        with self.assertRaisesRegex(ValueError, 'raw directory'):
            retention.retained_raw(self.root, self.entry)
        raw.symlink_to(raw.with_name('saved-raw'))
        with self.assertRaisesRegex(ValueError, 'raw directory'):
            retention.retained_raw(self.root, self.entry)
        for name in ('../bundle.json', '/absolute', 'missing.profraw'):
            with self.assertRaisesRegex(ValueError, 'unsafe'):
                retention.verify_raw_file(raw, name, 'digest')

    def test_cli_register_dry_run_apply_and_failure_exit(self):
        with patch.object(retention, 'ROOT', self.base), \
             patch('sys.stdout', new_callable=io.StringIO), \
             patch.object(retention, 'in_use', return_value=False):
            (self.base / 'storage-maintenance').mkdir()
            with patch('sys.argv', ['capture-cache', '--register', str(self.root)]):
                retention.main()
            with patch('sys.argv', ['capture-cache']):
                retention.main()
            self.assertTrue(self.target.is_dir())
            self.evidence.write_bytes(b'corrupt')
            with patch('sys.argv', ['capture-cache', '--apply']):
                with self.assertRaises(SystemExit) as error:
                    retention.main()
            self.assertEqual(error.exception.code, 1)
            self.evidence.write_bytes(b'original counters')
            with patch('sys.argv', ['capture-cache', '--apply']):
                retention.main()
            self.assertFalse(self.target.exists())


if __name__ == '__main__':
    unittest.main()

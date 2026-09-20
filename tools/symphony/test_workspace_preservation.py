"""Evidence is durable before cleanup; partial failures preserve originals."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('preserve', Path(__file__).with_name('preserve_workspace.py'))
preserve = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preserve)


class PreservationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.workspace = self.root / 'GH-1'
        self.workspace.mkdir()
        self.archive = self.root / 'retained'
        (self.workspace / 'result.json').write_bytes(b'evidence')
        self.file = {'path': 'result.json', 'sha256': hashlib.sha256(b'evidence').hexdigest()}
        self.manifest([self.file])

    def manifest(self, files, **extra):
        (self.workspace / preserve.MANIFEST).write_text(json.dumps({'schema': 'symphony-evidence/v1', 'files': files, **extra}))

    def run_preservation(self):
        return preserve.preserve(self.workspace, self.archive)

    def test_durable_idempotent_copy_survives_original_removal(self):
        receipt = self.run_preservation()
        self.assertEqual(receipt, self.run_preservation())
        (self.workspace / 'result.json').unlink()
        self.assertEqual((receipt.parent / '0000.evidence').read_bytes(), b'evidence')
        self.assertEqual(json.loads(receipt.read_text())['files'][0]['sha256'], self.file['sha256'])

    def test_missing_manifest_file_or_hash_mismatch_does_not_authorize_cleanup(self):
        for value in [None, 'wrong']:
            path = self.workspace / 'result.json'
            path.unlink(missing_ok=True)
            if value is not None:
                path.write_text(value)
            with self.assertRaises((OSError, ValueError)):
                self.run_preservation()
            self.assertTrue(self.workspace.is_dir())
            self.assertFalse(list(self.archive.glob('*/receipt.json')))
        (self.workspace / preserve.MANIFEST).unlink()
        with self.assertRaises(FileNotFoundError):
            self.run_preservation()

    def test_retry_after_interruption_and_completed_archive_corruption(self):
        with patch.object(preserve.os, 'rename', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError):
                self.run_preservation()
        self.assertEqual((self.workspace / 'result.json').read_bytes(), b'evidence')
        receipt = self.run_preservation()
        (receipt.parent / '0000.evidence').write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            self.run_preservation()
        self.assertTrue((self.workspace / 'result.json').exists())

    def test_source_changes_after_completed_copy_still_block_cleanup(self):
        self.run_preservation()
        (self.workspace / 'result.json').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'mismatch'):
            self.run_preservation()

    def test_paths_symlinks_and_in_workspace_archive_are_rejected(self):
        for name in ['../result.json', '/etc/passwd', 'nested/../result.json', './result.json']:
            self.manifest([{**self.file, 'path': name}])
            with self.assertRaises(ValueError):
                self.run_preservation()
        (self.workspace / 'link').symlink_to(self.root, target_is_directory=True)
        self.manifest([{**self.file, 'path': 'link/GH-1/result.json'}])
        with self.assertRaises(OSError):
            self.run_preservation()
        self.manifest([self.file])
        with self.assertRaises(ValueError):
            preserve.preserve(self.workspace, self.workspace / 'retained')

    def test_fsync_failure_and_manifest_change_leave_originals(self):
        with patch.object(preserve.os, 'fsync', side_effect=OSError('disk failure')):
            with self.assertRaises(OSError):
                self.run_preservation()
        original = preserve.copy_evidence
        def changing(*args):
            result = original(*args)
            self.manifest([], empty_reason='changed')
            return result
        with patch.object(preserve, 'copy_evidence', side_effect=changing):
            with self.assertRaisesRegex(ValueError, 'manifest changed'):
                self.run_preservation()
        self.assertEqual((self.workspace / 'result.json').read_bytes(), b'evidence')

    def test_empty_requires_explicit_reason_and_duplicates_are_rejected(self):
        self.manifest([])
        with self.assertRaises(ValueError):
            self.run_preservation()
        self.manifest([self.file, self.file])
        with self.assertRaises(ValueError):
            self.run_preservation()
        self.manifest([], empty_reason='no evidence produced; never dispatched')
        self.assertTrue(self.run_preservation().exists())


if __name__ == '__main__':
    unittest.main()

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import fixed_workspace as fw


class FixedWorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.repo = self.base / 'repo'
        self.repo.mkdir()
        fw.git(self.repo, 'init', '--quiet')
        fw.git(self.repo, 'config', 'user.name', 'Fixture')
        fw.git(self.repo, 'config', 'user.email', 'fixture@example.invalid')
        (self.repo / 'tracked').write_text('baseline')
        fw.git(self.repo, 'add', '.')
        fw.git(self.repo, 'commit', '--quiet', '-m', 'fixture')
        self.slot = self.base / 'validation'

    def sync(self):
        with fw.lease(self.slot):
            return fw.synchronize(self.repo, self.slot)

    def test_thirty_reuses_preserve_identity_cache_and_exact_source(self):
        inode = None
        for turn in range(30):
            (self.repo / 'tracked').write_text(str(turn))
            (self.repo / 'tracked').chmod(0o755 if turn % 2 else 0o644)
            added = self.repo / 'added'
            if turn % 2:
                added.write_text('new input')
            elif added.exists():
                added.unlink()
            root, hashes = self.sync()
            inode = inode or root.stat().st_ino
            self.assertEqual(root.stat().st_ino, inode)
            self.assertEqual(fw.sources(self.repo), fw.sources(root))
            self.assertEqual(set(hashes), set(fw.sources(self.repo)))
            cache = self.slot / 'target'
            cache.mkdir(exist_ok=True)
            (cache / 'retained').write_text('cache')
        self.assertEqual((self.slot / 'target/retained').read_text(), 'cache')
        self.assertEqual(len(list(self.slot.glob('workspace*'))), 1)

    def test_deleted_tracked_file_and_new_commit(self):
        self.sync()
        (self.repo / 'tracked').unlink()
        root, hashes = self.sync()
        self.assertFalse((root / 'tracked').exists())
        self.assertEqual(hashes, {})
        (self.repo / 'next').write_text('next')
        fw.git(self.repo, 'add', '-A')
        fw.git(self.repo, 'commit', '--quiet', '-m', 'next')
        self.sync()
        self.assertEqual(fw.git(root, 'rev-parse', 'HEAD'), fw.git(self.repo, 'rev-parse', 'HEAD'))

    def test_unchanged_dirty_source_preserves_mtime_and_inode(self):
        (self.repo / 'tracked').write_text('uncommitted source')
        root, _ = self.sync()
        target = root / 'tracked'
        os.utime(target, ns=(1000000000, 1000000000))
        original = target.stat()
        for _ in range(3):
            self.sync()
            self.assertEqual(target.stat().st_mtime_ns, original.st_mtime_ns)
            self.assertEqual(target.stat().st_ino, original.st_ino)
        self.assertEqual(target.read_text(), 'uncommitted source')

    def test_lock_rejects_second_writer_and_releases_after_failure(self):
        with self.assertRaisesRegex(RuntimeError, 'cancel'):
            with fw.lease(self.slot):
                with self.assertRaises(BlockingIOError):
                    with fw.lease(self.slot):
                        self.fail('second writer admitted')
                raise RuntimeError('cancel')
        with fw.lease(self.slot):
            pass

    def test_rejects_alias_relative_path_and_symlink_lock(self):
        alias = self.base / 'alias'
        alias.symlink_to(self.repo, target_is_directory=True)
        for path in (alias, Path('relative')):
            with self.assertRaises(ValueError):
                fw.canonical(path)
        self.slot.mkdir()
        (self.slot / 'writer.lock').symlink_to(self.repo / 'tracked')
        with self.assertRaises(OSError):
            with fw.lease(self.slot):
                pass
        self.assertEqual((self.repo / 'tracked').read_text(), 'baseline')

    def test_rejects_source_symlinks_and_nonregular_files(self):
        (self.repo / 'alias').symlink_to(self.repo / 'tracked')
        with self.assertRaises(ValueError):
            fw.sources(self.repo)
        (self.repo / 'alias').unlink()
        (self.repo / 'escape').symlink_to(self.base / 'outside')
        with self.assertRaisesRegex(ValueError, 'escapes'):
            fw.sources(self.repo)
        with self.assertRaises(ValueError):
            fw.fingerprint(self.repo)

    def test_rejects_existing_unowned_checkout(self):
        self.slot.mkdir()
        root = self.slot / 'workspace'
        root.mkdir()
        (root / 'valuable').write_text('keep')
        with self.assertRaisesRegex(ValueError, 'unowned'):
            self.sync()
        self.assertEqual((root / 'valuable').read_text(), 'keep')

    def test_rejects_changed_owned_source_without_overwriting(self):
        root, _ = self.sync()
        (root / 'tracked').write_text('external edit')
        with self.assertRaisesRegex(ValueError, 'outside synchronization'):
            self.sync()
        self.assertEqual((root / 'tracked').read_text(), 'external edit')

    def test_rejects_unknown_files_without_deleting(self):
        root, _ = self.sync()
        (root / 'valuable').write_text('keep')
        with self.assertRaisesRegex(ValueError, 'unowned validation source'):
            self.sync()
        self.assertEqual((root / 'valuable').read_text(), 'keep')

    def test_rejects_rebound_repository_and_symlink_owner(self):
        self.sync()
        owner = self.slot / 'owner.json'
        record = json.loads(owner.read_text())
        record['repository'] = '/wrong'
        owner.write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, 'ownership changed'):
            self.sync()
        owner.unlink()
        owner.symlink_to(self.repo / 'tracked')
        with self.assertRaisesRegex(ValueError, 'symlink validation'):
            self.sync()

    def test_rejects_symlink_inventory_and_destination(self):
        root, _ = self.sync()
        inventory = self.slot / 'source-inputs.json'
        inventory.unlink()
        inventory.symlink_to(self.repo / 'tracked')
        with self.assertRaisesRegex(ValueError, 'symlink source inventory'):
            self.sync()
        (root / 'tracked').unlink()
        (root / 'tracked').symlink_to(self.repo / 'tracked')
        with self.assertRaisesRegex(ValueError, 'symlink validation destination'):
            fw.copy_sources(self.repo, root, {'tracked': {}})

    def test_interrupted_initialization_is_not_silently_adopted(self):
        with fw.lease(self.slot):
            fw.initialize(self.repo, self.slot)
        with self.assertRaisesRegex(ValueError, 'unowned validation source'):
            self.sync()

    def test_source_change_during_copy_blocks_receipt(self):
        original = fw.copy_sources
        def change(repository, root, current):
            original(repository, root, current)
            (repository / 'tracked').write_text('concurrent edit')
        with patch.object(fw, 'copy_sources', side_effect=change):
            with self.assertRaisesRegex(ValueError, 'source changed during'):
                self.sync()
        self.assertFalse((self.slot / 'source-inputs.json').exists())

    def test_remote_revision_reuses_slot_and_preserves_development_changes(self):
        revision=fw.git(self.repo,'rev-parse','HEAD').decode().strip()
        (self.repo/'tracked').write_text('dirty local')
        (self.repo/'local-only').write_text('keep')
        root,_=self.sync();inode=root.stat().st_ino
        with fw.lease(self.slot):
            root,inputs=fw.synchronize_revision(self.repo,self.slot,revision)
        self.assertEqual(root.stat().st_ino,inode)
        self.assertEqual((root/'tracked').read_text(),'baseline')
        self.assertFalse((root/'local-only').exists())
        self.assertEqual((self.repo/'tracked').read_text(),'dirty local')
        self.assertEqual((self.repo/'local-only').read_text(),'keep')
        self.sync()
        self.assertEqual((root/'tracked').read_text(),'dirty local')
        with self.assertRaises(ValueError):fw.synchronize_revision(self.repo,self.slot,'HEAD')
        with patch.object(fw,'sources',return_value={'extra':{}}),patch.object(fw,'verify_previous'):
            with self.assertRaisesRegex(ValueError,'exact commit'):fw.synchronize_revision(self.repo,self.slot,revision)


if __name__ == '__main__':
    unittest.main()

import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import bounded_layout as layout


class LayoutTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.repo = self.root / 'source'; self.repo.mkdir()
        for name in ('validation', 'cache', 'tmp', 'evidence'):
            (self.root / name).mkdir()
        for name, value in (('VOLUME', self.root), ('REPOSITORY', self.repo)):
            p = patch.object(layout, name, value); p.start(); self.addCleanup(p.stop)
        p = patch.object(Path, 'is_mount', lambda path: path in (self.root, self.repo))
        p.start(); self.addCleanup(p.stop)

    def test_paths_and_lock_are_fixed_but_receipts_are_independent(self):
        with layout.lease(self.repo):
            first = layout.new_run()
            second = layout.new_run()
            self.assertNotEqual(first, second)
            self.assertEqual(first.parent, second.parent)
            self.assertNotEqual(layout.target(), layout.target('instrumented'))
            self.assertEqual(layout.target(), layout.target())
            self.assertEqual(layout.slot(), self.root / 'validation/gate')
            with self.assertRaises(BlockingIOError):
                with layout.lease(self.repo): self.fail('second writer admitted')
        with layout.lease(self.repo): pass

    def test_missing_mount_wrong_repository_and_cache_kind_fail_closed(self):
        with patch.object(Path, 'is_mount', return_value=False):
            with self.assertRaises(ValueError): layout.ensure(self.repo)
        with self.assertRaises(ValueError): layout.ensure(self.root)
        with self.assertRaises(ValueError): layout.target('new-attempt')
        with patch.object(layout,'CACHE_DOMAIN','wrong'):
            with self.assertRaises(ValueError):layout.target()
        with patch.object(layout,'CACHE_DOMAIN','remote'):
            self.assertIn('cargo-remote-normal',str(layout.target()))

    def test_repository_alias_and_storage_escape_are_rejected(self):
        alias = self.root / 'alias'; alias.symlink_to(self.repo)
        with patch.object(layout, 'REPOSITORY', alias), patch.object(Path, 'is_mount', return_value=True):
            with self.assertRaisesRegex(ValueError, 'outside bounded'): layout.ensure(alias)
        (self.root / 'cache').rmdir()
        (self.root / 'cache').symlink_to(self.root / 'tmp')
        with self.assertRaisesRegex(ValueError, 'storage path changed'): layout.ensure(self.repo)


if __name__ == '__main__': unittest.main()

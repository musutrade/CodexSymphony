"""A new run must reuse compiler storage instead of deleting it at handoff."""
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
import bounded_layout as layout


class CacheTests(unittest.TestCase):
    def test_thirty_runs_keep_compiler_cache_in_one_fixed_slot(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(layout,'VOLUME',Path(directory).resolve()),patch.object(layout,'CACHE_DOMAIN','local'):
            target=layout.target();cached=target/'compiled-object';cached.write_bytes(b'reusable')
            before=cached.stat()
            for _ in range(30):
                run=layout.new_run()
                self.assertNotEqual(run,layout.target())
                self.assertEqual(cached.read_bytes(),b'reusable')
                self.assertEqual(cached.stat().st_ino,before.st_ino)
            self.assertEqual(len(list((Path(directory)/'cache').iterdir())),1)
            self.assertFalse(any(p.name=='target' for p in (Path(directory)/'evidence').rglob('*')))

    def test_cache_symlink_cannot_adopt_evidence(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(layout,'VOLUME',Path(directory).resolve()),patch.object(layout,'CACHE_DOMAIN','local'):
            root=Path(directory).resolve();evidence=root/'evidence';evidence.mkdir()
            (root/'cache').symlink_to(evidence)
            with self.assertRaises(ValueError):layout.target()
            self.assertEqual(list(evidence.iterdir()),[])


if __name__=='__main__':unittest.main()

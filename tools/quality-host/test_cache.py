import tempfile
from pathlib import Path
import unittest
from run import prune_build_cache

class CacheTests(unittest.TestCase):
    def test_only_build_cache_is_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'target').mkdir();(root/'target/cache').write_text('rebuildable')
            for name in ['http-server','raw-object','receipt.json']: (root/name).write_text('retained')
            prune_build_cache(root)
            self.assertFalse((root/'target').exists())
            self.assertEqual((root/'http-server').read_text(),'retained')
            self.assertEqual((root/'raw-object').read_text(),'retained')
            self.assertEqual((root/'receipt.json').read_text(),'retained')
    def test_cache_symlink_cannot_delete_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'evidence').mkdir();(root/'target').symlink_to(root/'evidence')
            with self.assertRaises(ValueError):prune_build_cache(root)
            self.assertTrue((root/'evidence').is_dir())

if __name__=='__main__':unittest.main()

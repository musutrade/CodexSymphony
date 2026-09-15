import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC=importlib.util.spec_from_file_location('installer',Path(__file__).parents[1]/'install_gate_plugins.py')
INSTALLER=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(INSTALLER)

class PackagePins(unittest.TestCase):
    def test_changed_archive_rejected_before_installation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);archive=root/'candidate.tgz';archive.write_bytes(b'tampered')
            record={'sha256':hashlib.sha256(b'approved').hexdigest()}
            with self.assertRaisesRegex(ValueError,'digest mismatch'):
                INSTALLER.install(record,archive,root/'installed',root/'bin')
            self.assertFalse((root/'installed').exists())

if __name__=='__main__': unittest.main()

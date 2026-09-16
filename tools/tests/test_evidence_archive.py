import importlib.util,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).parents[1]))
import archive_gate_evidence as archive

class EvidenceArchive(unittest.TestCase):
    def fixture(self,root):
        run=root/'runs/run-0123456789ab';payload=run/'probes/backend';payload.mkdir(parents=True)
        (payload/'raw').write_bytes(b'evidence'*1024);(payload/'raw').chmod(0o664)
        (payload/'capture.stdout').write_text('diagnostic')
        (run/'report.json').write_text('report')
        return run,payload
    def test_round_trip_preserves_bytes_modes_and_online_diagnostics(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(archive.os.path,'ismount',return_value=True):
            root=Path(tmp);run,payload=self.fixture(root)
            result=archive.archive_run(run,root/'cold',busy=lambda _:False)
            self.assertFalse((payload/'raw').exists());self.assertEqual((payload/'capture.stdout').read_text(),'diagnostic')
            self.assertEqual((run/'report.json').read_text(),'report')
            restored=archive.restore(run,root/'restored')
            self.assertEqual((restored/'raw').read_bytes(),b'evidence'*1024)
            self.assertEqual((restored/'raw').stat().st_mode & 0o777,0o664)
            self.assertEqual(archive.archive_run(run,root/'cold',busy=lambda _:False),result)
    def test_corrupt_archive_rejected_and_active_source_kept(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(archive.os.path,'ismount',return_value=True):
            root=Path(tmp);run,payload=self.fixture(root)
            with self.assertRaises(RuntimeError):archive.archive_run(run,root/'cold',busy=lambda _:True)
            self.assertTrue((payload/'raw').exists())
            result=archive.archive_run(run,root/'cold',busy=lambda _:False)
            Path(result['archive']).write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError,'checksum'):archive.restore(run,root/'restored')
            self.assertFalse((root/'restored').exists())
    def test_failed_verification_keeps_originals(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(archive.os.path,'ismount',return_value=True):
            root=Path(tmp);run,payload=self.fixture(root)
            with patch.object(archive,'verify',side_effect=ValueError('bad archive')):
                with self.assertRaises(ValueError):archive.archive_run(run,root/'cold',busy=lambda _:False)
            self.assertTrue((payload/'raw').exists());self.assertFalse((run/'archive.json').exists())
    def test_symlink_source_rejected(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(archive.os.path,'ismount',return_value=True):
            root=Path(tmp);run,payload=self.fixture(root);(payload/'link').symlink_to(root)
            with self.assertRaises(ValueError):archive.archive_run(run,root/'cold',busy=lambda _:False)
            self.assertTrue((payload/'raw').exists())
if __name__=='__main__':unittest.main()

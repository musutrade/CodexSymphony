import json,os,sys,tempfile,time,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).parents[1]))
import compact_gate_evidence as c

class Compact(unittest.TestCase):
    def fixture(self,base):
        run=base/'run-0123456789ab';run.mkdir()
        (run/'source-inputs.json').write_text('{"commit":"abc"}')
        (run/'http-server').write_bytes(b'binary')
        raw=run/'probes/backend/raw';raw.mkdir(parents=True);(raw/'object').write_bytes(b'rebuildable')
        report=run/'workspace/.harness-gate/reports';(report/'evidence').mkdir(parents=True)
        (report/'test_result.json').write_text('{"passed":true}')
        (report/'evidence/detail.json').write_text('{"signed":"original"}')
        return run,report
    def test_preserve_reports_and_verified_details_discard_only_rebuildable(self):
        with tempfile.TemporaryDirectory() as tmp:
            run,report=self.fixture(Path(tmp));c.compact(run,busy=lambda _:False)
            self.assertTrue((report/'test_result.json').exists());self.assertFalse((run/'http-server').exists())
            self.assertFalse((report/'evidence').exists())
            record=json.loads((run/'compact-retention.json').read_text())
            c.verify(run/'review-record.tar.gz',record['files'])
            self.assertIn('workspace/.harness-gate/reports/evidence/detail.json',record['files'])
    def test_active_and_verification_failure_preserve_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            run,_=self.fixture(Path(tmp));self.assertTrue(c.compact(run,busy=lambda _:True)['deferred'])
            with patch.object(c,'verify',side_effect=ValueError('invalid')):
                with self.assertRaises(ValueError):c.compact(run,busy=lambda _:False)
            self.assertTrue((run/'http-server').exists())
    def test_crash_resume_does_not_replace_original_review_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            run,report=self.fixture(Path(tmp))
            with patch.object(c,'finish_compaction',side_effect=RuntimeError('crash')):
                with self.assertRaises(RuntimeError):c.compact(run,busy=lambda _:False)
            digest=c.sha(run/'review-record.tar.gz')
            c.shutil.rmtree(report/'evidence')
            c.compact(run,busy=lambda _:False)
            self.assertEqual(c.sha(run/'review-record.tar.gz'),digest)
    def test_symlink_and_nested_mount_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            run,_=self.fixture(Path(tmp));p=run/'probes/backend/raw'
            with patch.object(Path,'is_mount',lambda x:x==p):
                with self.assertRaises(ValueError):c.compact(run,busy=lambda _:False)
            (p/'link').symlink_to('/etc/passwd')
            with self.assertRaises(ValueError):c.compact(run,busy=lambda _:False)
    def test_cold_ttl_budget_and_manifest_retention(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(c.os.path,'ismount',return_value=True):
            root=Path(tmp);now=time.time()
            for i,age in [(1,0),(2,2),(3,8*86400)]:
                p=root/f'run-{i:012x}-backend.tar.gz';p.write_bytes(b'0123456789');os.utime(p,(now-age,now-age))
            receipt=root/'run-000000000003-backend.json';receipt.write_text('retained hashes')
            c.expire_cold(root,now,apply=True,budget=10)
            self.assertTrue((root/'run-000000000001-backend.tar.gz').exists())
            self.assertFalse((root/'run-000000000002-backend.tar.gz').exists())
            self.assertFalse((root/'run-000000000003-backend.tar.gz').exists())
            self.assertTrue(receipt.exists());self.assertEqual(len(list(root.glob('*.expired.json'))),2)
    def test_hot_retention_obeys_count_age_and_byte_budget(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(c.os.path,'ismount',return_value=True):
            root=Path(tmp);runs=root/'gate-host/runs';runs.mkdir(parents=True);cold=root/'cold';cold.mkdir();now=time.time()
            for i,age in [(1,7200),(2,7300),(3,2*86400)]:
                run=runs/f'run-{i:012x}';run.mkdir();identity=run/'source-inputs.json';identity.write_text('{}');os.utime(identity,(now-age,now-age));(run/'http-server').write_bytes(b'0123456789')
            with patch.object(c,'compact',side_effect=lambda r:{'run':r.name}) as compact:
                c.maintain(root,cold,apply=True,now=now,keep=2,budget=10)
                self.assertEqual([call.args[0].name for call in compact.call_args_list],['run-000000000002','run-000000000003'])
if __name__=='__main__':unittest.main()

import hashlib,json,os,sys,tempfile,time,unittest
from pathlib import Path
from unittest.mock import patch,Mock
sys.path.insert(0,str(Path(__file__).parents[1]))
import retire_pr_attempts as r

class Retention(unittest.TestCase):
    def fixture(self,root,identity,status,pr=39,age=600,has_run=True):
        job=root/'remote-gate/jobs'/identity.replace('/','-');job.mkdir(parents=True)
        source=job/'source';source.mkdir();(source/'dependency').write_bytes(b'large rebuildable dependency')
        receipt={'identity':identity,'source_sha':'a'*40,'finished':True,'status':status}
        if has_run:
            sequence=int(identity.split('/')[0])*10+int(identity.split('/')[1])
            run=root/'gate-host/runs'/f'run-{sequence:012x}';run.mkdir(parents=True)
            (run/'source-inputs.json').write_text('{}');(run/'large-binary').write_bytes(b'discard')
            report=run/'workspace/.harness-gate/reports/test_result.json';report.parent.mkdir(parents=True);report.write_text('{"passed":true}')
            (run/'capture.stderr').write_text('failed assertion before retry')
            receipt.update(run=str(run),report_sha256=r.sha(report))
        p=job/'receipt.json';p.write_text(json.dumps(receipt));os.utime(p,(time.time()-age,time.time()-age))
        (job/'attempt-context.json').write_text(json.dumps({'identity':identity,'source_sha':'a'*40,'pr':pr,'repository':r.REPOSITORY}))
        return job,receipt
    def run_policy(self,root,**kwargs):
        with patch.object(r.subprocess,'run',return_value=Mock(returncode=0,stdout='a'*40)):
            return r.maintain_attempts(root,root/'cold',apply=True,busy=lambda _:False,**kwargs)
    def test_failures_and_preparation_interruptions_retire_after_same_pr_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir()
            failed,receipt=self.fixture(root,'10/1','FAIL',age=1000)
            interrupted,_=self.fixture(root,'11/1','interrupted',age=900,has_run=False)
            success,accepted=self.fixture(root,'12/1','PASS',age=600)
            self.run_policy(root)
            self.assertFalse((failed/'source').exists());self.assertFalse((interrupted/'source').exists())
            run=Path(receipt['run']);summary=json.loads((run/'superseded-attempt.json').read_text())
            self.assertTrue(summary['complete']);self.assertEqual(summary['superseded_by'],'12/1')
            self.assertIn('failed assertion',summary['logs']['run/capture.stderr']['text'])
            self.assertFalse((run/'large-binary').exists())
            self.assertTrue((Path(accepted['run'])/'workspace/.harness-gate/reports/test_result.json').exists())
            self.assertFalse((success/'source').exists())
    def test_documentation_success_cleans_clone_without_retiring_full_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir()
            full,accepted=self.fixture(root,'10/1','PASS',age=1000)
            cancelled,_=self.fixture(root,'11/1','CANCELLED',age=900)
            docs,receipt=self.fixture(root,'12/1','PASS',age=600,has_run=False)
            report=docs/'documentation-result.json';report.write_text('{"scope":"documentation"}')
            receipt.update(scope='documentation',report_sha256=r.sha(report))
            path=docs/'receipt.json';path.write_text(json.dumps(receipt));os.utime(path,(time.time()-600,)*2)
            self.run_policy(root)
            self.assertFalse((docs/'source').exists());self.assertTrue(report.exists())
            self.assertTrue((Path(accepted['run'])/'workspace/.harness-gate/reports/test_result.json').exists())
            self.assertTrue((cancelled/'source').exists())
            self.fixture(root,'13/1','PASS',age=400)
            self.run_policy(root)
            self.assertFalse((cancelled/'source').exists())

    def test_other_pr_active_recent_and_later_failure_are_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir()
            other,_=self.fixture(root,'10/1','FAIL',pr=40,age=1000)
            active,receipt=self.fixture(root,'11/1','FAIL',age=1000);receipt['finished']=False;(active/'receipt.json').write_text(json.dumps(receipt))
            later,_=self.fixture(root,'13/1','FAIL',age=400)
            self.fixture(root,'12/1','PASS',age=600)
            self.run_policy(root)
            for job in (other,active,later):self.assertTrue((job/'source').exists())
    def test_same_workflow_retries_keep_only_latest_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir()
            first,_=self.fixture(root,'10/1','FAIL',age=1200)
            second,_=self.fixture(root,'10/2','interrupted',age=1000,has_run=False)
            success,receipt=self.fixture(root,'10/3','PASS',age=600)
            self.run_policy(root)
            for job in (first,second):
                self.assertEqual(json.loads((job/'superseded-attempt.json').read_text())['superseded_by'],'10/3')
            self.assertTrue((Path(receipt['run'])/'large-binary').exists())
    def test_busy_attempt_and_invalid_success_proof_cannot_delete(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir();failed,_=self.fixture(root,'10/1','FAIL',age=1000);success,receipt=self.fixture(root,'12/1','PASS')
            with patch.object(r.subprocess,'run',return_value=Mock(returncode=0,stdout='a'*40)):
                r.maintain_attempts(root,root/'cold',apply=True,busy=lambda _:True)
            self.assertTrue((failed/'source').exists())
            (Path(receipt['run'])/'workspace/.harness-gate/reports/test_result.json').write_text('modified')
            self.run_policy(root);self.assertTrue((failed/'source').exists())
    def test_crash_resume_preserves_original_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'cold').mkdir();failed,_=self.fixture(root,'10/1','FAIL',age=1000);self.fixture(root,'12/1','PASS')
            original=r.shutil.rmtree
            with patch.object(r.shutil,'rmtree',side_effect=RuntimeError('interrupted')):
                with self.assertRaises(RuntimeError):self.run_policy(root)
            before=json.loads((failed/'superseded-attempt.json').read_text())['logs']
            self.run_policy(root)
            self.assertEqual(json.loads((failed/'superseded-attempt.json').read_text())['logs'],before)
    def test_ambiguous_commit_and_wrong_sha_not_grouped(self):
        with tempfile.TemporaryDirectory() as tmp:
            job=Path(tmp);receipt={'identity':'10/1','source_sha':'a'*40}
            run={'head_sha':'a'*40,'repository':{'full_name':r.REPOSITORY},'event':'pull_request','path':'.github/workflows/quality.yml','pull_requests':[{'number':n,'base':{'repo':{'full_name':r.REPOSITORY}}} for n in (39,40)]}
            self.assertIsNone(r.context(job,receipt,lookup=lambda _:run))
            run['head_sha']='b'*40;self.assertIsNone(r.context(job,receipt,lookup=lambda _:run))
    def test_dependency_links_are_unlinked_but_mounts_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'source';source.mkdir();external=root/'external';external.write_text('keep');(source/'link').symlink_to(external)
            r.disposable(source);r.shutil.rmtree(source);self.assertEqual(external.read_text(),'keep')
            source.mkdir();nested=source/'nested';nested.mkdir()
            with patch.object(Path,'is_mount',lambda p:p==nested):
                with self.assertRaises(ValueError):r.disposable(source)
    def test_stacked_pr_commit_uses_verified_workflow_head_ref(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt={'identity':'10/1','source_sha':'a'*40}
            run={'head_sha':'a'*40,'repository':{'full_name':r.REPOSITORY},'event':'pull_request','path':'.github/workflows/quality.yml','head_branch':'symphony/GH-18','pull_requests':[]}
            prs=[{'number':n,'head':{'ref':branch},'base':{'repo':{'full_name':r.REPOSITORY}}} for n,branch in [(39,'symphony/GH-18'),(40,'followup')]]
            value=r.context(Path(tmp),receipt,lookup=lambda path:prs if path.endswith('/pulls') else run)
            self.assertEqual(value['pr'],39)
if __name__=='__main__':unittest.main()

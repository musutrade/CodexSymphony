import importlib.util,json,tempfile,unittest
from pathlib import Path
s=importlib.util.spec_from_file_location('diagnostics',Path(__file__).parents[1]/'export_gate_diagnostics.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
class DiagnosticsTests(unittest.TestCase):
 def test_redaction(self):
  value='postgres://name:secret@host/db Authorization: Bearer abc123 github_pat_123456 password=hide\n-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----'
  result=m.redact(value)
  for secret in ('name:secret','abc123','github_pat_123456','hide','\nsecret\n'):self.assertNotIn(secret,result)
 def test_exact_head_and_attempt_and_readonly_mount_destination(self):
  with tempfile.TemporaryDirectory() as tmp:
   base=Path(tmp);client=base/'symphony/gh16-environment/client';client.mkdir(parents=True)
   (base/'symphony/WORKFLOW.lifecycle.md.handoffs.json').write_text(json.dumps({'entries':{'16':{'handoff':{'head_sha':'a'*40}}}}))
   for attempt,head in [('100-1','b'*40),('100-2','a'*40)]:
    job=base/'remote-gate/jobs'/attempt;job.mkdir(parents=True)
    (job/'receipt.json').write_text(json.dumps({'source_sha':head,'identity':attempt.replace('-','/'),'finished':True,'status':'FAIL'}))
    (job/'gate.stderr').write_text('specific failure')
   m.export(base)
   root=client/'host-diagnostics'
   self.assertFalse((root/'100-1.json').exists())
   self.assertEqual(json.loads((root/'100-2.json').read_text())['logs']['gate.stderr'],'specific failure')
   self.assertEqual(json.loads((root/'latest.json').read_text())['source_sha'],'a'*40)
 def test_log_symlink_not_exported(self):
  with tempfile.TemporaryDirectory() as tmp:
   p=Path(tmp);(p/'secret').write_text('sensitive');(p/'log').symlink_to(p/'secret')
   self.assertIsNone(m.read_log(p/'log'))
 def test_long_log_truncated_after_redaction(self):
  with tempfile.TemporaryDirectory() as tmp:
   p=Path(tmp)/'log';p.write_text('x'*(m.LIMIT+100)+'\npassword=secret\nFAILED')
   result=m.read_log(p);self.assertTrue(result.endswith('FAILED'));self.assertNotIn('secret',result)
if __name__=='__main__':unittest.main()

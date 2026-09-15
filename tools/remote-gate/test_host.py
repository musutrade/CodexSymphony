import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from host import validate_run,check_files

class TrustedRun(unittest.TestCase):
    def setUp(self):
        self.config={'repository':'musutrade/CodexSymphony'}
        self.run={'repository':{'full_name':self.config['repository']},'head_repository':{'full_name':self.config['repository']},
                  'path':'.github/workflows/quality.yml','event':'pull_request','head_sha':'a'*40,'id':12,'run_attempt':1}
    def test_exact_attempt_identity(self):
        self.assertEqual(validate_run(self.run,self.config),'12/1')
        self.run['run_attempt']=2
        self.assertEqual(validate_run(self.run,self.config),'12/2')
    def test_fork_and_wrong_workflow_are_rejected(self):
        for field,value in [('head_repository',{'full_name':'attacker/fork'}),('path','.github/workflows/fake.yml'),('head_sha','main'),('run_attempt',0)]:
            changed=self.run|{field:value}
            with self.subTest(field=field),self.assertRaises(ValueError): validate_run(changed,self.config)
    def test_policy_and_symlink_changes_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);p=root/'policy';p.write_text('approved')
            pins={'policy':hashlib.sha256(p.read_bytes()).hexdigest()};check_files(root,pins)
            p.write_text('weakened')
            with self.assertRaises(ValueError):check_files(root,pins)
            p.unlink();target=root/'target';target.write_text('approved');p.symlink_to(target)
            with self.assertRaises(ValueError):check_files(root,pins)

if __name__=='__main__':unittest.main()

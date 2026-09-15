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

class InstallationTokens(unittest.TestCase):
    def test_scoped_token_reuse_and_refresh(self):
        from unittest.mock import patch
        import github
        config={'app_id':7,'app_key':'/host-only/key','repository':'owner/repo'}
        github._tokens.clear()
        def response(path,token,method='GET',body=None):
            if path.endswith('/installation'):return {'app_id':7,'id':9}
            self.assertEqual(body['permissions']['checks'],'write')
            return {'token':'synthetic-fixture-token','expires_at':'2099-01-01T00:00:00Z'}
        with patch.object(github.time,'time',return_value=1000) as clock,patch.object(github.subprocess,'check_output',return_value=b'synthetic-signature'),patch.object(github,'request',side_effect=response) as request:
            self.assertEqual(github.installation_token(config),'synthetic-fixture-token')
            self.assertEqual(github.installation_token(config),'synthetic-fixture-token')
            self.assertEqual(request.call_count,2)
            github.installation_token(config|{'repository':'owner/second'})
            self.assertEqual(request.call_count,4)
            clock.return_value=1241
            github.installation_token(config)
            self.assertEqual(request.call_count,6)
        github._tokens.clear()

    def test_failed_grant_is_not_cached(self):
        from unittest.mock import patch
        import github
        github._tokens.clear()
        config={'app_id':7,'app_key':'/host-only/key','repository':'owner/repo'}
        with patch.object(github.subprocess,'check_output',return_value=b'synthetic-signature'),patch.object(github,'request',side_effect=RuntimeError('unavailable')):
            with self.assertRaises(RuntimeError):github.installation_token(config)
        self.assertEqual(github._tokens,{})

if __name__=='__main__':unittest.main()

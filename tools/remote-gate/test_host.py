import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from host import validate_run,check_files

class TrustedRun(unittest.TestCase):
    def test_cross_device_dependencies_copy_and_other_errors_propagate(self):
        import errno,os
        from unittest.mock import patch
        from host import link_or_copy
        with tempfile.TemporaryDirectory() as tmp:
            source=Path(tmp)/'source';destination=Path(tmp)/'destination'
            source.write_bytes(b'dependency');source.chmod(0o755)
            with patch('host.os.link',side_effect=OSError(errno.EXDEV,'cross-device')):
                link_or_copy(source,destination)
            self.assertEqual(destination.read_bytes(),source.read_bytes())
            self.assertEqual(destination.stat().st_mode,source.stat().st_mode)
            with patch('host.os.link',side_effect=OSError(errno.EACCES,'denied')):
                with self.assertRaises(OSError):link_or_copy(source,Path(tmp)/'denied')
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

class InterruptedRecovery(unittest.TestCase):
    def test_finished_actions_receipt_is_reconciled_once_without_success(self):
        import json
        from unittest.mock import patch
        import host
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);p=home/'jobs/12-1/receipt.json';p.parent.mkdir(parents=True)
            p.write_text(json.dumps({'identity':'12/1','check_id':9,'source_sha':'a'*40,'finished':False}))
            with patch.object(host,'request') as request:
                host.reconcile_interrupted({'repository':'owner/repo'},home,'fixture')
                self.assertEqual(request.call_args.args[3]['conclusion'],'failure')
                self.assertEqual(json.loads(p.read_text())['status'],'interrupted')
                host.reconcile_interrupted({'repository':'owner/repo'},home,'fixture')
                self.assertEqual(request.call_count,1)

class DeploymentTransition(unittest.TestCase):
    def test_only_whole_approved_snapshots_match(self):
        import json
        from host import approved_deployment
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); candidates=[]
            for version in ('old','new'):
                pins={}
                for name in ('protected','collector','policy'):
                    content=(version+name).encode();(root/name).write_bytes(content)
                    pins[name]=hashlib.sha256(content).hexdigest()
                approval=root/(version+'.json')
                approval.write_text(json.dumps({'trusted_files':{'collector':pins['collector']},
                    'config_files':{'policy':pins['policy']},'version':version}))
                candidates.append({'protected_files':{'protected':pins['protected']},'gate_approval':str(approval)})
            config=candidates[1]|{'previous_deployments':[candidates[0]]}
            for version in ('new','old'):
                for name in ('protected','collector','policy'):(root/name).write_text(version+name)
                self.assertEqual(approved_deployment(root,config)[1]['version'],version)
            (root/'protected').write_text('newprotected')
            with self.assertRaisesRegex(ValueError,'complete reviewed'):approved_deployment(root,config)
            (root/'collector').write_text('newcollector');(root/'policy').write_text('newpolicy')
            (root/'protected').unlink();(root/'protected').symlink_to(root/'collector')
            with self.assertRaises(ValueError):approved_deployment(root,config)
            self.assertEqual(config['gate_approval'],candidates[1]['gate_approval'])

    def test_unknown_deployment_fields_are_not_configuration_overrides(self):
        from host import approved_deployment
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);approval=root/'approval.json'
            approval.write_text('{"trusted_files": {"missing":"digest"}, "config_files": {}}')
            config={'gate_approval':str(approval),'protected_files':{},
                    'previous_deployments':[{'protected_files':{},'gate_approval':str(approval),'repository':'other'}]}
            with self.assertRaisesRegex(ValueError,'invalid reviewed'):approved_deployment(root,config)


class GateTimeBudget(unittest.TestCase):
    def test_timeout_default_configuration_and_invalid_values(self):
        from host import gate_timeout
        self.assertEqual(gate_timeout({}),1500)
        self.assertEqual(gate_timeout({'gate_timeout_seconds':2400}),2400)
        for value in (0,-1,True,'2400',1.5,None):
            with self.subTest(value=value),self.assertRaises(ValueError):
                gate_timeout({'gate_timeout_seconds':value})

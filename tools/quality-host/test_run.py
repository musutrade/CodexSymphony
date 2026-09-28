from contextlib import ExitStack,nullcontext
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import sys
import bounded_layout as layout
import fixed_workspace as fw
import run as host


class HostTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.base=Path(self.tmp.name).resolve();self.repo=self.base/'repo';self.root=self.base/'root';self.run=self.base/'run';self.home=self.base/'host';self.slot=self.base/'slot'
        for p in (self.repo,self.root,self.run,self.home,self.slot):p.mkdir()
        self.args=SimpleNamespace(repository=self.repo,profile='ci',bootstrap=False,revision=None,approval=self.base/'approval.json')
        self.patch(host,'HOME',self.home);self.patch(layout,'slot',lambda:self.slot)
        env=patch.dict(os.environ,os.environ.copy());env.start();self.addCleanup(env.stop)

    def patch(self,owner,name,value):
        p=patch.object(owner,name,value);p.start();self.addCleanup(p.stop)

    def test_arguments_snapshot_and_generated_cleanup(self):
        with patch.object(sys,'argv',['run','--repository',str(self.repo)]):
            self.assertEqual(host.arguments().repository,self.repo)
        with patch.object(fw,'synchronize',return_value=(self.root,{'a':'hash'})):
            self.assertEqual(host.snapshot(self.repo,self.run),(self.root,{'a':'hash'}))
        with patch.object(fw,'synchronize_revision',return_value=(self.root,{})) as sync:
            host.snapshot(self.repo,self.run,'revision');sync.assert_called_once_with(self.repo,self.slot,'revision')
        generated=self.root/'.harness-gate/runtime';generated.mkdir(parents=True);(generated/'old').write_text('old')
        host.reset_generated(self.root);self.assertFalse(generated.exists())
        generated.symlink_to(self.repo)
        with self.assertRaises(ValueError):host.reset_generated(self.root)
        self.assertTrue(self.repo.exists())

    def test_approval_requires_repository_pins_configuration_and_trusted_sources(self):
        record={'host_release':str(Path(host.__file__).parent),'execution_version':host.EXECUTION_VERSION,'repository':str(self.repo),'runtime_files':{},'config_files':{},'trusted_files':{}}
        self.args.approval.write_text(json.dumps(record))
        with patch.object(host.contract,'load',return_value={}),patch.object(host.contract,'check_files'),patch.object(host.contract,'test_environment',return_value={}),patch.object(host.contract,'tool_path',return_value='/fixture'),patch.object(host,'check_pins'),patch.object(host,'configuration_files',return_value={}),patch.object(host,'trusted_files',return_value={}):
            self.assertEqual(host.approved(self.args,self.repo),record)
            for field,value in [('host_release','/wrong'),('repository','/wrong'),('config_files',{'drift':'hash'}),('trusted_files',{'drift':'hash'})]:
                self.args.approval.write_text(json.dumps(dict(record,**{field:value})))
                with self.assertRaises(ValueError):host.approved(self.args,self.repo)
            self.args.bootstrap=True
            with self.assertRaises(ValueError):host.approved(self.args,self.repo)
            self.args.approval.unlink();self.assertIsNone(host.approved(self.args,self.repo))

    def test_baseline_is_copied_only_for_explicit_bootstrap_and_otherwise_pinned(self):
        (self.repo/'api').mkdir();(self.repo/'api/baseline.json').write_text('{}')
        self.args.bootstrap=True
        baseline=host.baseline_for(self.args,None,self.repo,self.run,'revision')
        self.args.bootstrap=False
        self.assertEqual(host.baseline_for(self.args,{'baseline':baseline},self.repo,self.run,'revision'),baseline)
        (self.run/'baseline.json').write_text('changed')
        with self.assertRaises(ValueError):host.baseline_for(self.args,{'baseline':baseline},self.repo,self.run,'revision')

    def test_candidate_retains_exact_tree_reports_and_rejects_every_failed_boundary(self):
        (self.root/'.harness-gate/reports').mkdir(parents=True)
        (self.root/'.harness-gate/reports/report.json').write_text('{}')
        for mode in ('policy','verify','source','hash','validation','bootstrap','normal'):
            with ExitStack() as stack:
                for owner,name,value in [(host.subprocess,'check_output','revision'),(host,'baseline_for',{'commit':'base'}),(host,'reset_generated',None),(host,'captures',({},{})),(host,'seal',None),(host,'configure',{}),(host,'configuration_files',{}),(host,'state',{}),(host,'provision',None),(host,'verify',1 if mode=='verify' else 0),(fw,'sources',{'changed':{'sha256':'hash'}} if mode=='source' else {}),(host.shutil,'copytree',None),(host,'accept_bootstrap',None)]:
                    stack.enter_context(patch.object(owner,name,return_value=value))
                self.args.bootstrap=mode=='bootstrap'
                approval={'config_files':{'wrong':'hash'}} if mode=='policy' else {'config_files':{}}
                if mode=='validation':
                    stack.enter_context(patch.object(fw,'sources',side_effect=[{}, {'different':{}}]))
                if mode in ('policy','verify','source','hash','validation'):
                    with self.assertRaises((ValueError,RuntimeError)):host.run_candidate(self.args,approval,self.repo,self.run,self.root,{'wrong':'hash'} if mode=='hash' else {})
                else:self.assertEqual(host.run_candidate(self.args,approval,self.repo,self.run,self.root,{}),{})

    def test_bootstrap_records_resulting_binding_and_preserves_existing_prior_approval(self):
        for root in (self.repo,self.root):
            (root/'.harness-gate/packs/backend').mkdir(parents=True)
        (self.root/'config').write_text('new config');(self.root/'.harness-gate/packs/backend/capabilities.json').write_text('{}')
        base=self.run/'baseline.json';base.write_text('{}')
        baseline={'path':str(base),'sha256':host.sha(base.read_bytes()),'commit':'base'}
        with patch.object(host,'trusted_files',return_value={}),patch.object(host,'runtime_pins',return_value={}):
            host.accept_bootstrap(self.args,self.repo,self.root,self.run,baseline,{'config':'hash'},{'backend':{}})
        self.assertEqual((self.repo/'config').read_text(),'new config')
        self.assertTrue((self.run/'bootstrap-source-change.json').exists())
        self.assertEqual(json.loads(self.args.approval.read_text())['execution_version'],3)

    def test_finish_archives_capture_and_distinguishes_bootstrap(self):
        import capture_handoff
        (self.run/'measurement-summary.json').write_text('{"coverage_and_crap":"PASS"}')
        (self.run/'tmp').mkdir();(self.run/'tmp/sccache-fixture.json').write_text('{}')
        pending=self.slot/'pending-gate.json'
        for bootstrap in (False,True):
            self.args.bootstrap=bootstrap
            with patch.object(capture_handoff,'complete') as complete,patch.object(fw,'sources',return_value={'input':{'sha256':'hash'}}):
                host.finish_capture(self.args,self.run,self.root,pending)
                complete.assert_called_once()
            self.assertEqual(json.loads((self.run/'complete-gate.json').read_text())['scope'],'bootstrap-diagnostic' if bootstrap else 'complete-local-isolated-gate')

    def test_only_active_approval_enables_cleanup(self):
        self.assertFalse(host.active_approval(self.args))
        (self.home/'approval.json').symlink_to(self.args.approval)
        self.assertTrue(host.active_approval(self.args))

    def test_main_keeps_shared_lease_and_blocks_unfinished_attempts(self):
        with patch('bounded_retention.maintain_locked'),patch.object(host,'active_approval',return_value=True),patch.object(host,'arguments',return_value=self.args),patch.object(layout,'lease',return_value=nullcontext()),patch.object(host,'approved',return_value={}),patch.object(layout,'new_run',return_value=self.run),patch.object(host,'snapshot',return_value=(self.root,{})),patch.object(host.contract,'fingerprint',return_value={}),patch.object(host,'run_candidate') as candidate,patch.object(host,'finish_capture'):
            host.main();candidate.assert_called_once()
            self.assertTrue((self.slot/'pending-gate.json').exists())
            with self.assertRaisesRegex(ValueError,'unfinished'):host.main()

    def test_remote_reset_cannot_touch_local_cache(self):
        with patch.object(layout,'CACHE_DOMAIN','local'):
            with self.assertRaises(ValueError):host.reset_remote_targets()
        with patch.object(layout,'CACHE_DOMAIN','remote'),patch.object(layout,'VOLUME',self.base):
            for kind in ('normal','instrumented'):(layout.target(kind)/'old').write_text('old')
            host.reset_remote_targets()
            for kind in ('normal','instrumented'):self.assertEqual(list(layout.target(kind).iterdir()),[])

    def test_remote_main_rejects_bootstrap_and_checks_frozen_source(self):
        self.args.revision='a'*40
        with patch.object(host,'arguments',return_value=self.args),patch.object(layout,'lease',return_value=nullcontext()),patch.object(host,'approved',return_value={}) as approved,patch.object(layout,'new_run',return_value=self.run),patch.object(host,'snapshot',return_value=(self.root,{})),patch.object(host.contract,'fingerprint',return_value={}),patch.object(host,'run_candidate'),patch.object(host,'finish_capture'),patch.object(host,'reset_remote_targets') as reset,patch.object(host,'active_approval',return_value=False):
            self.args.bootstrap=True
            with self.assertRaisesRegex(ValueError,'bootstrap'):host.main()
            self.args.bootstrap=False
            host.main();approved.assert_called_once_with(self.args,self.repo,source=self.root);reset.assert_called_once()
        layout.CACHE_DOMAIN='local'


if __name__=='__main__':unittest.main()

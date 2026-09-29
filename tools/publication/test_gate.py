import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publication_gate', Path(__file__).with_name('gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class PublicationTests(unittest.TestCase):
    def test_git_tree_includes_edits_additions_deletions_and_modes_without_index_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.DEVNULL)
            git('init');git('config','user.name','Test');git('config','user.email','test@example.invalid')
            (root/'code').write_text('initial');git('add','.');git('commit','-m','base')
            index = (root/'.git/index').read_bytes()
            initial = gate.source_tree(root)
            (root/'code').write_text('changed');edited = gate.source_tree(root)
            (root/'code').chmod(0o755);executable = gate.source_tree(root)
            (root/'added').write_text('new');added = gate.source_tree(root)
            (root/'code').unlink();deleted = gate.source_tree(root)
            self.assertEqual(len({initial,edited,executable,added,deleted}),5)
            self.assertEqual((root/'.git/index').read_bytes(),index)

    def test_repository_filters_cannot_transform_validated_bytes_or_execute_on_host(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            def git(*args):
                return subprocess.check_output(['git','-C',str(root),*args],stderr=subprocess.DEVNULL)
            git('init');git('config','user.name','Test');git('config','user.email','test@example.invalid')
            (root/'code').write_bytes(b'original\r\n');git('add','.');git('commit','-m','base')
            before=gate.source_tree(root)
            (root/'.gitattributes').write_text('code filter=host-command text eol=lf\n')
            git('config','filter.host-command.clean','touch FILTER_EXECUTED; cat')
            git('config','core.fsmonitor','touch FSMONITOR_EXECUTED')
            actual=gate.source_tree(root)
            self.assertNotEqual(before,actual)
            self.assertFalse((root/'FILTER_EXECUTED').exists())
            self.assertFalse((root/'FSMONITOR_EXECUTED').exists())
            (root/'code').write_bytes(b'original\n')
            self.assertNotEqual(actual,gate.source_tree(root))

    def test_admission_requires_a_standing_ledger_pass_for_the_exact_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();report=root/'report.json'
            report.write_text(json.dumps({'passed':True,'evidence_complete':True}))
            identity={'tree':'a'*40,'environment':'environment-A','approval':'policy-A'}
            commit='c'*40;values=gate.ledger_inputs(identity,commit);store=root/'ledger'
            attempt=gate.ledger.begin(store,values)
            validation=gate.ledger.record_pass(store,attempt,values,{'report':str(report)},{})
            receipt={'status':'PASS','scope':'complete-local-isolated-gate','inputs':identity,'commit':commit,
                     'validation_id':validation,'report':str(report),'report_sha256':gate.ledger.file_digest(report)}
            with patch.object(gate,'LEDGER',store),patch.object(gate,'head_commit',return_value=commit):
                with patch.object(gate,'inputs',return_value=identity):
                    self.assertEqual(gate.admit(root,receipt,'a'*40)['validation_id'],validation)
                    for candidate in [{},receipt|{'status':'FAIL'},receipt|{'scope':'hook'},receipt|{'validation_id':'f'*64},
                                      receipt|{'report_sha256':'0'*64},receipt|{'report':str(root/'other.json')}]:
                        with self.subTest(candidate=candidate),self.assertRaises(ValueError):gate.admit(root,candidate,'a'*40)
                    with self.assertRaises(ValueError):gate.admit(root,receipt,'b'*40)
                    with patch.object(gate,'head_commit',return_value='d'*40):
                        with self.assertRaisesRegex(ValueError,'commit changed'):gate.admit(root,receipt,'a'*40)
                for field in identity:
                    with patch.object(gate,'inputs',return_value=identity|{field:'changed'}):
                        with self.assertRaises(ValueError):gate.admit(root,receipt|{'inputs':identity|{field:'changed'}},'a'*40)
                with patch.object(gate,'inputs',return_value=identity):
                    report.write_text(json.dumps({'passed':False}))
                    with self.assertRaises(gate.ledger.LedgerError):gate.admit(root,receipt,'a'*40)
                    report.write_text(json.dumps({'passed':True,'evidence_complete':True}))
                    gate.ledger.revoke(store,validation,'withdrawn')
                    with self.assertRaisesRegex(ValueError,'blocked'):gate.admit(root,receipt,'a'*40)

    def test_incomplete_report_is_rejected_even_when_its_digest_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            report=Path(directory)/'report.json';report.write_text('{"passed":true}')
            with self.assertRaisesRegex(ValueError,'incomplete'):gate.check_report(report)

    def test_head_commit_reads_the_exact_checkout_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            def git(*args):
                return subprocess.check_output(['git','-C',str(root),*args],stderr=subprocess.DEVNULL,text=True).strip()
            git('init');git('config','user.name','Test');git('config','user.email','test@example.invalid')
            (root/'code').write_text('x');git('add','.');git('commit','-m','base')
            self.assertEqual(gate.head_commit(root),git('rev-parse','HEAD'))

    def test_commit_parents_reads_the_validated_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo=Path(temporary)
            def git(*args):
                return subprocess.check_output(['git','-C',str(repo),*args],text=True).strip()
            git('init','-q');git('config','user.email','fixture@example.invalid');git('config','user.name','Fixture')
            git('commit','--allow-empty','-qm','root');root=git('rev-parse','HEAD')
            git('commit','--allow-empty','-qm','child');child=git('rev-parse','HEAD')
            self.assertEqual(gate.commit_parents(repo,child),[root])
            self.assertEqual(gate.commit_parents(repo,root),[])
            with patch.object(gate.subprocess,'check_output',return_value='f'*40+'\n'):
                with self.assertRaisesRegex(ValueError,'parents unavailable'):gate.commit_parents(repo,child)

    def test_workspace_cannot_escape_issue_directory(self):
        with self.assertRaises(ValueError):gate.workspace('../GH-1')


class FixedPublicationTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        self.base=Path(temporary.name).resolve();self.state=self.base/'publication';self.state.mkdir()
        self.approval=self.base/'approval.json'
        self.value={'execution_version':3,'repository':'/home/gem/CodexSymphony','host_release':str(self.base/'host')}
        self.approval.write_text(json.dumps(self.value))
        for name,value in [('STATE',self.state),('APPROVAL',self.approval),('WORKSPACES',self.base/'workspaces')]:
            item=patch.object(gate,name,value);item.start();self.addCleanup(item.stop)

    def test_fixed_workspace_requires_host_issue_binding_and_real_mounts(self):
        path=self.state/'active-workspace.json'
        value={'issue':'GH-90','repository':'/home/gem/CodexSymphony'};path.write_text(json.dumps(value))
        from types import SimpleNamespace
        original=Path.stat
        def mounted_stat(path,*args,**kwargs):
            if str(path) in ['/home/gem/CodexSymphony','/mnt/dev-ssd/codexsymphony-bounded/data']:
                return SimpleNamespace(st_dev=7)
            return original(path,*args,**kwargs)
        with patch.object(Path,'is_mount',return_value=True),patch.object(Path,'stat',mounted_stat):
            self.assertEqual(gate.workspace('GH-90'),Path(value['repository']))
            with self.assertRaisesRegex(ValueError,'not assigned'):gate.workspace('GH-91')
            with self.assertRaises(ValueError):gate.fixed_binding('GH-90',self.value|{'repository':'/other'})
        with patch.object(Path,'is_mount',return_value=False):
            with self.assertRaisesRegex(ValueError,'mounts'):gate.workspace('GH-90')
        path.unlink();path.symlink_to(self.approval)
        with self.assertRaisesRegex(ValueError,'aliased'):gate.workspace('GH-90')
        self.approval.write_text('{"execution_version":2}')
        gate.WORKSPACES.mkdir();root=gate.WORKSPACES/'GH-90';root.mkdir()
        self.assertEqual(gate.workspace('GH-90'),root)
        root.rmdir();root.symlink_to(self.base)
        with self.assertRaisesRegex(ValueError,'unsafe'):gate.workspace('GH-90')

    def test_snapshot_uses_archived_exact_files_and_modes_and_approved_reader(self):
        run=self.base/'run';run.mkdir();host=self.base/'host';host.mkdir()
        source=host/'fixed_workspace.py';source.write_text("def sources(root):return {'file':{'sha256':'hash','mode':436}}\n")
        approval=self.value|{'runtime_files':{str(source):gate.hashlib.sha256(source.read_bytes()).hexdigest()}}
        archive=run/'source.tar.gz';archive.write_bytes(b'archive')
        manifest={'sha256':gate.hashlib.sha256(archive.read_bytes()).hexdigest(),'inputs':{'file':{'sha256':'hash','mode':436}}}
        (run/'source-archive.json').write_text(json.dumps(manifest));(run/'source-inputs.json').write_text('{"file":"hash"}')
        self.assertTrue(gate.snapshot_matches(run,self.base,approval))
        manifest['inputs']['file']['mode']=493;(run/'source-archive.json').write_text(json.dumps(manifest))
        self.assertFalse(gate.snapshot_matches(run,self.base,approval))
        source.write_text('changed')
        with self.assertRaisesRegex(ValueError,'reader'):gate.snapshot_matches(run,self.base,approval)
        archive.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'archive'):gate.snapshot_matches(run,self.base,approval)
        with patch.object(gate,'source_tree',return_value='tree'):
            self.assertTrue(gate.snapshot_matches(run,self.base,{'execution_version':2}))

    def validation_fixture(self):
        run=self.base/'run';run.mkdir();(run/'reports').mkdir()
        (run/'reports/test_result.json').write_text(json.dumps({'passed':True,'evidence_complete':True,'source_identity':'working-tree:'+'c'*40}))
        (run/'environment.json').write_text('{"fingerprint":"environment"}')
        for name in gate.REQUIRED_EVIDENCE[3][2:]+('complete-gate.json',):
            (run/name).write_text(name)
        self.write_context(run,'c'*40)
        identity={'tree':'a'*40,'environment':'environment','approval':'approval'}
        response={'status':'PASS','scope':'complete-local-isolated-gate','run':str(run)}
        def execute(*args,**kwargs):kwargs['stdout'].write(json.dumps(response)+'\n')
        from contextlib import ExitStack
        stack=ExitStack();self.addCleanup(stack.close)
        for owner,name,value in [(gate,'workspace',self.base),(gate,'inputs',identity),(gate,'snapshot_matches',True),
             (gate,'head_commit','c'*40),(gate,'commit_parents',['p'*40]),(gate.contract,'load',{}),(gate.contract,'tool_path','/usr/bin:/bin'),
             (gate.contract,'test_environment',{})]:
            stack.enter_context(patch.object(owner,name,return_value=value))
        stack.enter_context(patch.object(gate,'LEDGER',self.state/'ledger'))
        stack.enter_context(patch.object(gate.subprocess,'run',side_effect=execute))
        return run,identity,response

    def write_context(self,run,requests,capture=None):
        (run/'requests.json').write_text(json.dumps({name:{'context':{'commit':requests}} for name in ['backend','frontend']}))
        (run/'test-capture.json').write_text(json.dumps({'context':{'commit':capture or requests}}))

    def receipt(self):
        return json.loads((self.state/'GH-90/receipt.json').read_text())

    def kinds(self):
        return [json.loads(line)['kind'] for line in (self.state/'ledger/events.jsonl').read_text().splitlines()]

    def test_validate_records_an_attempt_bound_pass_with_minimal_evidence(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        run,identity,_=self.validation_fixture()
        gate.validate('GH-90')
        receipt=self.receipt();self.assertEqual(receipt['status'],'PASS')
        self.assertEqual(receipt['report'],str(run/'reports/test_result.json'))
        self.assertEqual(self.kinds(),['start','pass'])
        record=gate.ledger.verify(gate.LEDGER,receipt['validation_id'],gate.ledger_inputs(identity,'c'*40))
        self.assertEqual(record['attempt'],receipt['attempt'])
        self.assertEqual(set(record['evidence']),{'report','complete-gate.json','test-capture.json',*gate.REQUIRED_EVIDENCE[3]})
        self.assertEqual((receipt['commit'],record['inputs']['commit']),('c'*40,'c'*40))
        self.assertEqual(record['details']['run'],str(run))
        self.assertEqual(record['details']['parents'],['p'*40])
        self.assertEqual(gate.admit(self.base,receipt,'a'*40)['validation_id'],receipt['validation_id'])

    def test_every_rejected_boundary_concludes_its_own_attempt_as_fail(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        run,identity,response=self.validation_fixture()
        gate.validate('GH-90');passed=self.receipt()['validation_id']
        with patch.object(gate,'snapshot_matches',return_value=False):
            with self.assertRaisesRegex(ValueError,'snapshot'):gate.validate('GH-90')
        receipt=self.receipt()
        self.assertEqual((receipt['status'],self.kinds()),('FAIL',['start','pass','start','fail']))
        self.assertIn('validation_id',receipt)
        with self.assertRaisesRegex(ValueError,'blocked'):gate.admit(self.base,self.receipt()|{'status':'PASS','validation_id':passed,'inputs':identity,'commit':'c'*40,'scope':'complete-local-isolated-gate','report':str(run/'reports/test_result.json'),'report_sha256':gate.ledger.file_digest(run/'reports/test_result.json')},'a'*40)
        with patch.object(gate,'inputs',side_effect=[identity,identity|{'tree':'b'*40}]):
            with self.assertRaisesRegex(ValueError,'changed during'):gate.validate('GH-90')
        response['status']='FAIL'
        with self.assertRaisesRegex(ValueError,'acceptance missing'):gate.validate('GH-90')
        response['status']='PASS';(run/'environment.json').write_text('{"fingerprint":"other"}')
        with self.assertRaisesRegex(ValueError,'fingerprints'):gate.validate('GH-90')
        (run/'environment.json').write_text('{"fingerprint":"environment"}');(run/'source.tar.gz').unlink()
        with self.assertRaisesRegex(ValueError,'mismatch'):gate.validate('GH-90')
        self.assertEqual(self.kinds().count('start'),self.kinds().count('pass')+self.kinds().count('fail'))
        self.assertEqual(self.receipt()['status'],'FAIL')

    def test_same_tree_with_another_head_or_evidence_commit_is_not_recorded_as_pass(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        run,identity,_=self.validation_fixture()
        report=run/'reports/test_result.json'
        # Same tree and environment throughout; only the commit identity differs.
        with patch.object(gate,'head_commit',side_effect=['c'*40,'d'*40]):
            with self.assertRaisesRegex(ValueError,'HEAD changed'):gate.validate('GH-90')
        for requests,capture,source in [('d'*40,None,'c'*40),('c'*40,'d'*40,'c'*40),('c'*40,None,'d'*40)]:
            with self.subTest(requests=requests,capture=capture,source=source):
                self.write_context(run,requests,capture)
                report.write_text(json.dumps({'passed':True,'evidence_complete':True,'source_identity':'working-tree:'+source}))
                with self.assertRaisesRegex(ValueError,'another commit'):gate.validate('GH-90')
        self.assertNotIn('pass',self.kinds())
        self.assertEqual(self.kinds(),['start','fail']*4)
        failed=gate.ledger.load_record(gate.LEDGER,self.receipt()['validation_id'])
        self.assertEqual(failed['inputs'],gate.ledger_inputs(identity,'c'*40))
        with self.assertRaisesRegex(ValueError,'blocked'):gate.ledger.current(gate.LEDGER,gate.ledger_inputs(identity,'d'*40))

    def test_non_v3_approval_does_not_read_v3_context_files(self):
        run=self.base/'run';run.mkdir()
        with patch.object(gate,'head_commit',return_value='c'*40):
            gate.check_commit(self.base,run,run/'absent.json',{'execution_version':2},'c'*40)
            with self.assertRaisesRegex(ValueError,'HEAD changed'):
                gate.check_commit(self.base,run,run/'absent.json',{'execution_version':2},'d'*40)

    def crash(self,identity,named=True):
        """Leave the state a killed validate leaves: RUNNING receipt with a pending attempt."""
        value=gate.ledger_inputs(identity,'c'*40)
        attempt=gate.ledger.begin(gate.LEDGER,value)
        (self.state/'GH-90').mkdir(parents=True,exist_ok=True)
        receipt={'status':'RUNNING','identity':value}|({'attempt':attempt} if named else {})
        gate.atomic(self.state/'GH-90/receipt.json',receipt)
        return value,attempt

    def terminated(self,state='inactive',group='/user.slice/validate.service',events='populated 0\nfrozen 0\n'):
        """Patch the unit, its cgroup subtree and the host leases to a settled, empty execution."""
        # A fresh hierarchy per call: events left by an earlier case must not leak into the next.
        cgroup=Path(tempfile.mkdtemp(dir=self.base,prefix='cgroup-'));self.cgroup=cgroup
        leases=self.base/'leases';leases.mkdir(exist_ok=True)
        if events is not None:
            (cgroup/group.lstrip('/')).mkdir(parents=True,exist_ok=True)
            (cgroup/group.lstrip('/')/'cgroup.events').write_text(events)
        paths=(leases/'coordination.lock',leases/'slot.lock')
        # Always regular files: a FIFO or removal left by an earlier case must not leak either.
        for path in paths:path.unlink(missing_ok=True);path.touch()
        self.bind_leases(paths)
        from contextlib import ExitStack
        stack=ExitStack()
        stack.enter_context(patch.object(gate,'CGROUP_ROOT',cgroup))
        stack.enter_context(patch.object(gate.subprocess,'check_output',
                                         return_value='ActiveState='+state+'\nControlGroup='+group+'\n'))
        return stack

    def test_recover_concludes_only_a_terminated_crashed_attempt_as_fail(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        _,identity,_=self.validation_fixture()
        gate.validate('GH-90');old=self.receipt()['validation_id']
        value,attempt=self.crash(identity)
        for state in ['active','deactivating','activating','']:
            with self.subTest(state=state),self.terminated(state):
                with self.assertRaisesRegex(ValueError,'unit is'):gate.recover('GH-90')
        # A failed unit can still leave children in its cgroup.
        with self.terminated('failed',events='populated 1\nfrozen 0\n'):
            with self.assertRaisesRegex(ValueError,'cgroup still has processes'):gate.recover('GH-90')
        self.assertEqual(self.kinds(),['start','pass','start'])
        with self.terminated('failed'):
            concluded=gate.recover('GH-90')
        self.assertEqual((concluded['status'],concluded['attempt'],concluded['identity']),('FAIL',attempt,value))
        record=gate.ledger.load_record(gate.LEDGER,concluded['validation_id'])
        self.assertEqual((record['outcome'],record['attempt'],record['inputs']),('fail',attempt,value))
        # Recovery never restores the old PASS; only a new complete Gate stands.
        with self.assertRaisesRegex(ValueError,'blocked'):gate.ledger.verify(gate.LEDGER,old,value)
        with self.terminated():
            with self.assertRaisesRegex(ValueError,'no single unconcluded'):gate.recover('GH-90')
        gate.validate('GH-90')
        self.assertEqual(gate.ledger.current(gate.LEDGER,value)['validation_id'],self.receipt()['validation_id'])

    def test_recover_requires_the_gate_execution_leases_to_be_free(self):
        self.validation_fixture()
        self.crash({'tree':'a'*40,'environment':'environment','approval':'approval'})
        with self.terminated():
            with (self.base/'leases/coordination.lock').open() as held:
                gate.fcntl.flock(held,gate.fcntl.LOCK_EX)
                with self.assertRaisesRegex(ValueError,'execution lease is held'):gate.recover('GH-90')
            with (self.state/'GH-90/lock').open('a') as held:
                gate.fcntl.flock(held,gate.fcntl.LOCK_EX)
                with self.assertRaises(BlockingIOError):gate.recover('GH-90')
        self.assertEqual(self.kinds(),['start'])

    def test_recover_requires_every_v3_lease_and_a_readable_cgroup_occupancy(self):
        self.validation_fixture()
        self.crash({'tree':'a'*40,'environment':'environment','approval':'approval'})
        for missing in ['coordination.lock','slot.lock']:
            with self.subTest(missing=missing),self.terminated():
                (self.base/'leases'/missing).unlink()
                with self.assertRaisesRegex(ValueError,'lease unavailable'):gate.recover('GH-90')
        with self.terminated():
            (self.base/'leases/slot.lock').unlink();os.mkfifo(self.base/'leases/slot.lock')
            with self.assertRaisesRegex(ValueError,'lease unavailable'):gate.recover('GH-90')
        for events in ['populated maybe\n','frozen 0\n']:
            with self.subTest(events=events),self.terminated(events=events):
                with self.assertRaisesRegex(ValueError,'unreadable cgroup'):gate.recover('GH-90')
        self.assertEqual(self.kinds(),['start'])
        # A group directory without v2 occupancy is missing evidence, not an empty group.
        with self.terminated(events=None):
            (self.cgroup/'user.slice/validate.service').mkdir(parents=True)
            with self.assertRaisesRegex(ValueError,'occupancy unavailable'):gate.recover('GH-90')
        self.assertEqual(self.kinds(),['start'])
        # A collected unit has no cgroup left; an empty group name means none was assigned.
        for group in ['/user.slice/gone.service','']:
            with self.subTest(group=group),self.terminated(group=group,events=None):
                self.assertFalse((self.cgroup/'user.slice').exists())
                self.assertFalse(gate.populated(group))
        with self.terminated(events='populated 1\n'):
            self.assertTrue(gate.populated('/user.slice/validate.service'))
        # A collected unit (no group directory left) with free leases concludes the crash.
        with self.terminated(events=None):
            self.assertEqual(gate.recover('GH-90')['status'],'FAIL')
        self.assertEqual(self.kinds(),['start','fail'])

    def bind_leases(self,paths,approval=None):
        approval=approval or json.loads(self.approval.read_text())
        binding=self.base/'host-leases.json'
        binding.write_text(json.dumps({'approval':gate.contract.digest(approval),'host_release':approval['host_release'],
                                       'leases':[str(p) for p in paths]}))
        item=patch.object(gate,'HOST_LEASES',binding);item.start();self.addCleanup(item.stop)
        return binding

    def test_lease_binding_must_match_the_current_installed_approval(self):
        paths=(self.base/'a.lock',self.base/'b.lock')
        binding=self.bind_leases(paths)
        self.assertEqual(gate.required_leases(),paths)
        self.approval.write_text(json.dumps(self.value|{'series':'changed'}))
        with self.assertRaisesRegex(ValueError,'another approval'):gate.required_leases()
        self.bind_leases(paths,self.value|{'series':'changed','host_release':'/elsewhere'})
        with self.assertRaisesRegex(ValueError,'another approval'):gate.required_leases()
        self.bind_leases((),self.value|{'series':'changed'})
        with self.assertRaisesRegex(ValueError,'empty'):gate.required_leases()
        binding.unlink()
        with self.assertRaisesRegex(ValueError,'binding missing'):gate.required_leases()
        self.approval.write_text(json.dumps(self.value|{'execution_version':2}))
        self.assertEqual(gate.required_leases(),())

    def test_recover_finds_an_attempt_the_receipt_never_learned(self):
        self.validation_fixture()
        value,attempt=self.crash({'tree':'a'*40,'environment':'environment','approval':'approval'},named=False)
        with self.terminated():
            self.assertEqual(gate.recover('GH-90')['attempt'],attempt)
        self.assertEqual(self.kinds(),['start','fail'])

    def test_recover_rejects_receipts_without_a_single_pending_attempt(self):
        self.validation_fixture()
        value,attempt=self.crash({'tree':'a'*40,'environment':'environment','approval':'approval'})
        gate.ledger.begin(gate.LEDGER,value)
        with self.terminated():
            gate.atomic(self.state/'GH-90/receipt.json',{'status':'RUNNING','identity':value})
            with self.assertRaisesRegex(ValueError,'no single'):gate.recover('GH-90')
            for receipt in [{'status':'RUNNING'},{'status':'MISSING'},{'identity':value,'attempt':attempt,'validation_id':'f'*64}]:
                with self.subTest(receipt=receipt):
                    gate.atomic(self.state/'GH-90/receipt.json',receipt)
                    with self.assertRaisesRegex(ValueError,'no single'):gate.recover('GH-90')
        self.assertEqual(self.kinds(),['start','start'])

    def test_a_pending_attempt_pointer_is_never_overwritten_by_start_or_validate(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        _,identity,_=self.validation_fixture()
        value,attempt=self.crash(identity)
        before=self.receipt()
        with patch.object(gate.subprocess,'run',return_value=subprocess.CompletedProcess([],3)) as run:
            with self.assertRaisesRegex(ValueError,'run recover'):gate.start('GH-90')
            self.assertEqual(len(run.call_args_list),1)  # only is-active; nothing was scheduled
        with self.assertRaisesRegex(ValueError,'run recover'):gate.validate('GH-90')
        self.assertEqual((self.receipt(),self.kinds()),(before,['start']))

    def test_a_refused_fail_keeps_the_identity_recover_needs(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        _,identity,_=self.validation_fixture()
        with patch.object(gate.ledger,'record_fail',side_effect=gate.ledger.LedgerError('tampered: x')):
            with patch.object(gate,'snapshot_matches',return_value=False):
                with self.assertRaisesRegex(ValueError,'snapshot'):gate.validate('GH-90')
        receipt=self.receipt()
        self.assertEqual((receipt['status'],receipt['identity']),('FAIL',gate.ledger_inputs(identity,'c'*40)))
        with self.assertRaisesRegex(ValueError,'run recover'):gate.validate('GH-90')
        with self.terminated():
            self.assertEqual(gate.recover('GH-90')['attempt'],receipt['attempt'])
        gate.validate('GH-90')
        self.assertEqual(self.receipt()['status'],'PASS')

    def test_orphaned_treats_only_an_absent_ledger_as_empty(self):
        self.validation_fixture()
        value=gate.ledger_inputs({'tree':'a'*40,'environment':'environment','approval':'approval'},'c'*40)
        self.assertEqual(gate.orphaned({'identity':value}),[])
        (self.state/'ledger').mkdir();(self.state/'ledger/lock').symlink_to(self.approval)
        with self.assertRaisesRegex(gate.ledger.LedgerError,'tampered'):gate.orphaned({'identity':value})

    def test_start_never_overwrites_a_receipt_while_a_validate_owns_execution(self):
        self.validation_fixture()
        (self.state/'GH-90').mkdir()
        gate.atomic(self.state/'GH-90/receipt.json',{'status':'RUNNING','identity':{'tree':'x'}})
        with (self.state/'GH-90/lock').open('a') as held:
            gate.fcntl.flock(held,gate.fcntl.LOCK_EX)
            with patch.object(gate.subprocess,'run') as run:
                self.assertEqual(gate.start('GH-90'),{'status':'RUNNING'})
            run.assert_not_called()
            # A second validate cannot claim execution and leaves the owner's receipt alone.
            with self.assertRaises(BlockingIOError):gate.validate('GH-90')
        self.assertEqual(self.receipt(),{'status':'RUNNING','identity':{'tree':'x'}})

    def test_validate_waits_for_scheduling_and_then_owns_the_receipt(self):
        import threading
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        self.validation_fixture()
        (self.state/'GH-90').mkdir()
        outcome=[]
        with (self.state/'GH-90/schedule.lock').open('a') as held:
            gate.fcntl.flock(held,gate.fcntl.LOCK_EX)
            gate.atomic(self.state/'GH-90/receipt.json',{'status':'PENDING'})
            worker=threading.Thread(target=lambda:outcome.append(gate.validate('GH-90')))
            worker.start();worker.join(0.3)
            self.assertTrue(worker.is_alive())
            self.assertEqual(self.receipt(),{'status':'PENDING'})
            self.assertFalse((self.state/'ledger/events.jsonl').exists())
        worker.join(10)
        self.assertEqual((outcome,self.receipt()['status'],self.kinds()),([None],'PASS',['start','pass']))

    def test_interleaved_starts_schedule_one_validation(self):
        import threading
        self.validation_fixture()
        scheduled=[];active=threading.Event()
        def systemctl(command,*args,**kwargs):
            if command[0]=='systemd-run':
                scheduled.append(command);active.set()
                return subprocess.CompletedProcess(command,0)
            return subprocess.CompletedProcess(command,0 if active.is_set() else 3)
        with patch.object(gate.subprocess,'run',side_effect=systemctl):
            threads=[threading.Thread(target=gate.start,args=('GH-90',)) for _ in range(6)]
            for thread in threads:thread.start()
            for thread in threads:thread.join()
        self.assertEqual((len(scheduled),self.receipt()),(1,{'status':'PENDING'}))

    def test_start_schedules_once_and_reports_an_active_unit(self):
        self.validation_fixture()
        with patch.object(gate.subprocess,'run',return_value=subprocess.CompletedProcess([],0)) as run:
            self.assertEqual(gate.start('GH-90'),{'status':'RUNNING'})
            self.assertEqual(len(run.call_args_list),1)
        self.assertFalse((self.state/'GH-90/receipt.json').exists())
        with patch.object(gate.subprocess,'run',return_value=subprocess.CompletedProcess([],3)) as run:
            self.assertEqual(gate.start('GH-90'),{'status':'RUNNING'})
        command=run.call_args_list[-1].args[0]
        self.assertEqual((command[:2],command[-3:]),(['systemd-run','--user'],['validate','--issue','GH-90']))
        self.assertIn('codexsymphony-prepublish-gh-90',command)
        self.assertEqual(self.receipt(),{'status':'PENDING'})

    def invoke(self,*argv):
        import io,contextlib
        output=io.StringIO()
        with patch.object(gate.sys,'argv',['gate.py',*argv]),contextlib.redirect_stdout(output):
            gate.main()
        return json.loads(output.getvalue()) if output.getvalue() else None

    def test_main_dispatches_every_operation(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        _,identity,_=self.validation_fixture()
        self.assertEqual(self.invoke('status','--issue','GH-90'),{'status':'MISSING'})
        self.assertIsNone(self.invoke('validate','--issue','GH-90'))
        passed=self.receipt()['validation_id']
        self.assertEqual(self.invoke('status','--issue','GH-90')['validation_id'],passed)
        self.assertEqual(self.invoke('check','--issue','GH-90','--tree','a'*40)['validation_id'],passed)
        with self.assertRaisesRegex(ValueError,'tree'):self.invoke('check','--issue','GH-90','--tree','b'*40)
        with patch.object(gate,'head_commit',return_value='d'*40):
            self.assertEqual(self.invoke('status','--issue','GH-90')['status'],'STALE')
        with patch.object(gate.subprocess,'run',return_value=subprocess.CompletedProcess([],0)):
            self.assertEqual(self.invoke('start','--issue','GH-90'),{'status':'RUNNING'})
        self.crash(identity)
        with self.terminated():
            self.assertEqual(self.invoke('recover','--issue','GH-90')['status'],'FAIL')
        self.assertIn('diagnostics',self.invoke('status','--issue','GH-90'))

    def test_failure_diagnostics_tail_logs_and_only_trusted_failed_phases(self):
        state=self.state/'GH-90';state.mkdir()
        runs=self.base/'gate-host/runs';trusted=runs/'run-1';trusted.mkdir(parents=True)
        other=self.base/'elsewhere/run-2';other.mkdir(parents=True)
        (trusted/'timings.jsonl').write_text('\n'.join(json.dumps(r) for r in [
            {'phase':'capture-all','status':'FAIL'},{'phase':'verify','status':'PASS'},
            {'phase':'Bad/..','status':'FAIL'},{'phase':'verify','status':'FAIL'}])+'\n')
        (trusted/'verify.stderr').write_text('x'*9000+'tail')
        (trusted/'capture-all.stdout').write_text('captured')
        (state/'gate.stdout').write_text('Retaining complete gate run: '+str(other)+'\nnoise\n'
                                         'Retaining complete gate run: '+str(trusted)+'\n')
        (state/'gate.stderr').write_text('e'*13000)
        with patch.object(gate,'BASE',self.base):
            logs=gate.diagnostics(state)
        self.assertEqual(set(logs),{'gate.stdout','gate.stderr','verify.stderr'})
        self.assertEqual((len(logs['gate.stderr']),len(logs['verify.stderr'])),(12000,8000))
        self.assertTrue(logs['verify.stderr'].endswith('tail'))
        self.assertEqual(gate.failed_phases(other),[])
        with patch.object(gate,'BASE',self.base):
            self.assertEqual(gate.diagnostics(self.base/'empty'),{})

    def test_failure_before_the_attempt_writes_no_ledger_event(self):
        self.validation_fixture()
        with patch.object(gate,'inputs',side_effect=ValueError('environment drift: x')):
            with self.assertRaisesRegex(ValueError,'drift'):gate.validate('GH-90')
        self.assertEqual((self.receipt()['status'],self.receipt()['attempt']),('FAIL',None))
        self.assertFalse((self.state/'ledger/events.jsonl').exists())

    def test_ledger_refusing_the_fail_leaves_the_attempt_pending_and_blocking(self):
        self.approval.write_text(json.dumps(self.value|{'repository':str(self.base)}))
        _,identity,_=self.validation_fixture()
        with patch.object(gate.ledger,'record_fail',side_effect=gate.ledger.LedgerError('tampered: x')):
            with patch.object(gate,'snapshot_matches',return_value=False):
                with self.assertRaisesRegex(ValueError,'snapshot'):gate.validate('GH-90')
        self.assertIn('ledger_error',self.receipt())
        self.assertEqual(self.kinds(),['start'])
        with self.assertRaisesRegex(ValueError,'blocked'):gate.ledger.current(gate.LEDGER,gate.ledger_inputs(identity,'c'*40))

    def test_non_v3_approval_binds_only_the_report_and_environment(self):
        run=self.base/'run';run.mkdir();(run/'environment.json').write_text('{}');(run/'report.json').write_text('{}')
        evidence=gate.retained_evidence(run,run/'report.json',{'execution_version':2})
        self.assertEqual(set(evidence),{'report','environment.json'})

    def test_publication_tree_rejects_unsafe_sources_and_bounds_fixed_workspace_temporary_files(self):
        self.assertIsNone(gate.temporary_root(self.base))
        root=Path('/home/gem/CodexSymphony')
        with patch.object(Path,'is_mount',return_value=True):
            self.assertEqual(gate.temporary_root(root),Path('/mnt/dev-ssd/codexsymphony-bounded/data/tmp'))
        with patch.object(Path,'is_mount',return_value=False):
            with self.assertRaisesRegex(ValueError,'temporary'):gate.temporary_root(root)
        subprocess.run(['git','init','--quiet',str(self.base)],check=True)
        code=self.base/'code';code.write_text('code')
        subprocess.run(['git','-C',str(self.base),'add','code'],check=True)
        code.unlink();code.symlink_to(self.approval)
        with self.assertRaisesRegex(ValueError,'unsafe source'):gate.source_tree(self.base)
        code.unlink();code.mkdir()
        with self.assertRaisesRegex(ValueError,'unsupported source'):gate.source_tree(self.base)


if __name__=='__main__':unittest.main()

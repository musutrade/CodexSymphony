import importlib.util
import json
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

    def test_missing_failed_stale_environment_wrong_tree_and_modified_report_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);report=root/'report.json'
            report.write_text(json.dumps({'passed':True,'evidence_complete':True}))
            identity={'tree':'a'*40,'environment':'environment-A','approval':'policy-A'}
            receipt={'status':'PASS','scope':'complete-local-isolated-gate','inputs':identity,
                     'report':str(report),'report_sha256':gate.hashlib.sha256(report.read_bytes()).hexdigest()}
            with patch.object(gate,'inputs',return_value=identity):
                self.assertEqual(gate.admit(root,receipt,'a'*40)['status'],'PASS')
                for candidate in [{},receipt|{'status':'FAIL'},receipt|{'scope':'hook'}]:
                    with self.assertRaises(ValueError):gate.admit(root,candidate,'a'*40)
                with self.assertRaises(ValueError):gate.admit(root,receipt,'b'*40)
            for field in identity:
                with patch.object(gate,'inputs',return_value=identity|{field:'changed'}):
                    with self.assertRaises(ValueError):gate.admit(root,receipt,'a'*40)
            report.write_text('modified')
            with patch.object(gate,'inputs',return_value=identity):
                with self.assertRaises(ValueError):gate.admit(root,receipt,'a'*40)

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

    def test_validate_accepts_exact_retained_report_and_rejects_all_stale_boundaries(self):
        run=self.base/'run';run.mkdir();(run/'reports').mkdir()
        (run/'reports/test_result.json').write_text('{"passed":true,"evidence_complete":true}')
        (run/'environment.json').write_text('{"fingerprint":"environment"}')
        identity={'tree':'a'*40,'environment':'environment','approval':'approval'}
        response={'status':'PASS','scope':'complete-local-isolated-gate','run':str(run)}
        def execute(*args,**kwargs):kwargs['stdout'].write(json.dumps(response)+'\n')
        from contextlib import ExitStack
        with ExitStack() as stack:
            for owner,name,value in [(gate,'workspace',self.base),(gate,'inputs',identity),(gate,'snapshot_matches',True),
                 (gate.contract,'load',{}),(gate.contract,'tool_path','/usr/bin:/bin'),(gate.contract,'test_environment',{})]:
                stack.enter_context(patch.object(owner,name,return_value=value))
            stack.enter_context(patch.object(gate.subprocess,'run',side_effect=execute))
            gate.validate('GH-90')
            receipt=json.loads((self.state/'GH-90/receipt.json').read_text());self.assertEqual(receipt['status'],'PASS')
            self.assertEqual(receipt['report'],str(run/'reports/test_result.json'))
            with patch.object(gate,'snapshot_matches',return_value=False):
                with self.assertRaisesRegex(ValueError,'snapshot'):gate.validate('GH-90')
            with patch.object(gate,'inputs',side_effect=[identity,identity|{'tree':'b'*40}]):
                with self.assertRaisesRegex(ValueError,'changed during'):gate.validate('GH-90')
            response['status']='FAIL'
            with self.assertRaisesRegex(ValueError,'acceptance missing'):gate.validate('GH-90')
            response['status']='PASS';(run/'environment.json').write_text('{"fingerprint":"other"}')
            with self.assertRaisesRegex(ValueError,'fingerprints'):gate.validate('GH-90')
            self.assertEqual(json.loads((self.state/'GH-90/receipt.json').read_text())['status'],'FAIL')

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

import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import fixed_checkout as fixed


class FixedRemoteTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name).resolve()
        self.git('init','--quiet');self.git('config','user.name','Fixture');self.git('config','user.email','fixture@example.invalid')
        (self.root/'file').write_text('reviewed')
        self.git('add','.');self.git('commit','--quiet','-m','fixture')
        self.revision=self.git('rev-parse','HEAD').decode().strip()
        self.approval=self.root/'approval.json'
        self.record={'execution_version':3,'repository':str(self.root),'trusted_files':{},'config_files':{}}
        self.approval.write_text(json.dumps(self.record))
        self.config={'gate_approval':str(self.approval),'protected_files':{'file':hashlib.sha256(b'reviewed').hexdigest()}}

    def git(self,*args):
        return subprocess.check_output(['git','-C',self.root,*args],stderr=subprocess.PIPE)

    def test_git_blob_ignores_dirty_files_and_rejects_missing_symlink_and_invalid_paths(self):
        (self.root/'file').write_text('uncommitted')
        self.assertEqual(fixed.blob(self.root,self.revision,'file'),b'reviewed')
        for rev,name in [('HEAD','file'),(self.revision,'../file'),(self.revision,'/file'),(self.revision,'missing')]:
            with self.assertRaises(ValueError):fixed.blob(self.root,rev,name)
        (self.root/'link').symlink_to('file');self.git('add','link');self.git('commit','--quiet','-m','link')
        with self.assertRaises(ValueError):fixed.blob(self.root,self.git('rev-parse','HEAD').decode().strip(),'link')

    def test_only_complete_bounded_approval_is_selected(self):
        self.assertEqual(fixed.select(self.root,self.revision,self.config)[1],self.record)
        for record in [dict(self.record,execution_version=2),dict(self.record,trusted_files={'file':'wrong'})]:
            self.approval.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError,'bounded reviewed'):fixed.select(self.root,self.revision,self.config)
        self.approval.write_text(json.dumps(self.record))
        self.config['protected_files']={'file':'wrong'}
        self.config['previous_deployments']=[{'bad':'shape'}]
        with self.assertRaisesRegex(ValueError,'invalid reviewed'):fixed.select(self.root,self.revision,self.config)

    def test_dependencies_are_compared_as_blobs_without_copying(self):
        deps=self.root/'web/angular/node_modules';deps.mkdir(parents=True)
        for name in ('package.json','package-lock.json'):(deps.parent/name).write_text('{}')
        config={'dependency_source':str(deps)}
        with patch.object(fixed,'blob',return_value=b'{}'):
            fixed.dependencies(self.root,self.revision,config)
            deps.rmdir()
            with self.assertRaisesRegex(ValueError,'fixed reviewed'):fixed.dependencies(self.root,self.revision,config)
        with patch.object(fixed,'blob',return_value=b'changed'):
            with self.assertRaisesRegex(ValueError,'host review'):fixed.dependencies(self.root,self.revision,config)

    def test_prepare_fetches_only_objects_and_checks_mounts(self):
        self.config['repository']='musutrade/CodexSymphony'
        run={'head_sha':self.revision}
        with self.assertRaisesRegex(ValueError,'mounts'):fixed.prepare(run,self.config)
        record=dict(self.record,repository='/home/gem/CodexSymphony')
        self.approval.write_text(json.dumps(record))
        from types import SimpleNamespace
        with patch.object(Path,'is_mount',return_value=True),patch.object(Path,'stat',return_value=SimpleNamespace(st_dev=1)),patch.object(fixed.subprocess,'run') as fetch,patch.object(fixed,'select',return_value=({},record)):
            root,result=fixed.prepare(run,self.config)
            self.assertEqual(root,Path(record['repository']));self.assertEqual(result,record)
            command=fetch.call_args.args[0]
            self.assertIn('fetch',command);self.assertNotIn('checkout',command);self.assertNotIn('clone',command)
            with patch.object(Path,'stat',side_effect=[SimpleNamespace(st_dev=1),SimpleNamespace(st_dev=2)]):
                with self.assertRaisesRegex(ValueError,'outside'):fixed.prepare(run,self.config)
            with self.assertRaisesRegex(ValueError,'invalid remote'):fixed.prepare({'head_sha':'HEAD'},self.config)


    def test_state_directory_is_bounded_and_legacy_approval_is_rejected(self):
        with patch.object(Path,'is_mount',return_value=True),patch.object(Path,'resolve',lambda path:path),patch.object(Path,'mkdir') as mkdir:
            self.assertEqual(fixed.state_home(self.config),Path('/mnt/dev-ssd/codexsymphony-bounded/data/evidence/remote-gate'))
            mkdir.assert_called_once()
        with patch.object(Path,'is_mount',return_value=False):
            with self.assertRaises(ValueError):fixed.state_home(self.config)
        with patch.object(Path,'is_mount',return_value=True),patch.object(Path,'resolve',return_value=Path('/wrong')):
            with self.assertRaisesRegex(ValueError,'aliased'):fixed.state_home(self.config)

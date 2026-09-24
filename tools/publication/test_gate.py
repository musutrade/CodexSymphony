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


if __name__=='__main__':unittest.main()

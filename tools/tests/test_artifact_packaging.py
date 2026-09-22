import copy
import hashlib
import gzip
import lzma
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('packaging',Path(__file__).parents[1]/'quality-host/artifact_packaging.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

RAW=json.dumps([{'function':str(i),'count':i % 7,'file':'source.rs'} for i in range(2000)]).encode()

class Packaging(unittest.TestCase):
    def fixture(self,root):
        refs=[]
        for i in range(80):
            name=f'{i:03}.gz';data=gzip.compress(RAW,mtime=0)
            (root/name).write_bytes(data)
            refs.append({'id':str(i),'path':name,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data),'media_type':'application/gzip','source':{'path':f'{i}.rs'},'context':{'commit':'exact-sha'}})
        return {'artifacts':refs,'collection':{'evidence':[{'artifacts':copy.deepcopy(refs),'metrics':[{'artifacts':[str(i) for i in range(80)]}]}]}}

    def test_lossless_encoding_preserves_distinct_source_descriptors(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);response=self.fixture(root);before=copy.deepcopy(response)
            result=m.compact_artifacts(response,root)
            self.assertEqual(len(list(root.iterdir())),80)
            self.assertEqual(len({r['path'] for r in result['artifacts']}),80)
            for ref in result['artifacts']+result['collection']['evidence'][0]['artifacts']:
                data=(root/ref['path']).read_bytes()
                self.assertEqual(hashlib.sha256(data).hexdigest(),ref['sha256'])
                raw=lzma.decompress(data) if ref['media_type']=='application/x-xz' else gzip.decompress(data)
                self.assertEqual(ref['media_type'],'application/x-xz')
                self.assertEqual(raw,RAW)
                original=before['artifacts'][int(ref['id'])]
                for key in ['path','sha256','bytes','media_type']:ref[key]=original[key]
            self.assertEqual(result,before)

    def test_tamper_and_nested_mismatch_fail_without_deletion(self):
        for bad in ['bytes','nested','symlink']:
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);response=self.fixture(root)
                if bad=='bytes':(root/'079.gz').write_bytes(b'changed')
                elif bad=='nested':response['collection']['evidence'][0]['artifacts'][0]['source']={'path':'wrong.rs'}
                else:
                    (root/'079.gz').unlink();(root/'079.gz').symlink_to(root/'000.gz')
                original=copy.deepcopy(response)
                with self.assertRaises(ValueError):m.compact_artifacts(response,root)
                self.assertEqual(len(list(root.iterdir())),80)
                self.assertEqual(response,original)

    def test_json_evidence_is_lossless_and_keeps_source_bindings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            (root/'coverage.json').write_bytes(RAW)
            ref={'id':'coverage','path':'coverage.json','sha256':hashlib.sha256(RAW).hexdigest(),
                 'bytes':len(RAW),'media_type':'application/json','source':{'path':'app.ts'},
                 'context':{'commit':'exact-sha'}}
            response={'artifacts':[ref], 'collection':{'evidence':[{'artifacts':[copy.deepcopy(ref)]}]}}
            result=m.compact_artifacts(response,root)
            packed=result['artifacts'][0]
            data=(root/packed['path']).read_bytes()
            self.assertEqual(lzma.decompress(data),RAW)
            self.assertLess(len(data),len(RAW))
            self.assertEqual(packed['sha256'],hashlib.sha256(data).hexdigest())
            self.assertEqual(packed['source'],{'path':'app.ts'})
            self.assertEqual(packed['context'],{'commit':'exact-sha'})
            self.assertEqual(packed,result['collection']['evidence'][0]['artifacts'][0])
            self.assertFalse((root/'coverage.json').exists())

class LauncherPackagingBoundary(unittest.TestCase):
    def test_api_contract_identity_is_not_reencoded(self):
        import sys
        from unittest.mock import patch
        host_sources=Path(__file__).parents[1]/'quality-host'
        with patch.object(sys,'path',[str(host_sources),*sys.path]):
            signing_spec=importlib.util.spec_from_file_location('packaging_signing',host_sources/'signing.py')
            signing=importlib.util.module_from_spec(signing_spec);signing_spec.loader.exec_module(signing)
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);plugin=root/'plugin';plugin.mkdir()
            (plugin/'plugin.py').write_text('')
            (plugin/'cli.cjs').write_text('')
            with patch.object(signing,'RUST',plugin), patch.object(signing,'TS',plugin), patch.object(signing,'HTTP',plugin), patch('shutil.which',return_value=sys.executable):
                for collector in ('backend','frontend','frontend-api'):
                    text=signing.runtime_launcher(root,collector).read_text()
                    compile(text,collector,'exec')
                    if collector=='frontend-api':
                        self.assertNotIn('compact_artifacts',text)
                        self.assertIn('os.execv(',text)
                    else:
                        self.assertIn('compact_artifacts(response',text)
                        self.assertNotIn('os.execv(',text)

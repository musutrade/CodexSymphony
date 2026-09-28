import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch
import urllib.error

import capture as c
import database_pool
import manual_capture
import manual_measure


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.base=Path(self.tmp.name).resolve()
        self.root=self.base/'source'; self.run=self.base/'run'; self.target=self.base/'target'
        for path in (self.root/'api',self.root/'web/angular',self.root/'tools/quality-host',self.run,self.target/'debug'):
            path.mkdir(parents=True,exist_ok=True)
        (self.target/'debug/codexsymphony-server').write_bytes(b'binary')
        (self.root/'api/openapi.json').write_text('{}')
        (self.root/'tools/quality-host/capture.py').write_text('fixture producer')

    def test_fixed_database_and_producer_cleanup_on_failure(self):
        with patch.object(database_pool,'acquire',return_value=('fixed','url')) as acquire:
            self.assertEqual(c.database(self.run,'-http',self.root),('fixed','url'))
            acquire.assert_called_once_with(self.run,'-http',self.root)
        for fail in (False,True):
            with patch.object(manual_capture,'backend') as backend, patch.object(manual_measure,'measure',return_value={'coverage_and_crap':'PASS'}),patch.object(manual_measure,'register'),patch.object(c,'command',return_value=['capture']) as command,patch.object(c,'run_logged'),patch.object(c,'database',return_value=('fixed','url')),patch.object(database_pool,'release') as release,patch.object(c,'capture_http',return_value=([], 'hash'),side_effect=RuntimeError('HTTP failed') if fail else None):
                if fail:
                    with self.assertRaises(RuntimeError): c.capture_producers(self.run,self.root,self.root)
                else:self.assertEqual(c.capture_producers(self.run,self.root,self.root),([], 'hash'))
                backend.assert_called_once_with(self.run,self.root)
                release.assert_called_once_with('fixed')
                self.assertEqual(command.call_args.kwargs['repository'],self.root)
        with patch.object(manual_capture,'backend'),patch.object(manual_measure,'measure',return_value={'coverage_and_crap':'FAIL'}),patch.object(manual_measure,'register'),patch.object(c,'database') as database:
            with self.assertRaisesRegex(ValueError,'coverage'): c.capture_producers(self.run,self.root,self.root)
            database.assert_not_called()

    def test_http_uses_fixed_target_and_stops_children_on_all_exits(self):
        for mode in ('success','spawn','lookup','timeout'):
            server=MagicMock(pid=123)
            if mode=='timeout':server.wait.side_effect=[subprocess.TimeoutExpired('fixture',5),None]
            with patch('bounded_layout.target',return_value=self.target),patch.object(c,'command',return_value=['fixture']) as command,patch.object(c,'run_logged'),patch.object(c.subprocess,'Popen',return_value=server,side_effect=OSError('spawn failed') if mode=='spawn' else None),patch.object(c,'wait_http_address',return_value='127.0.0.1:1'),patch.object(c,'capture_http_observations',return_value=[]),patch.object(c.os,'killpg',side_effect=ProcessLookupError() if mode=='lookup' else None) as kill:
                if mode=='spawn':
                    with self.assertRaises(OSError):c.capture_http_session(self.run,self.root,'fixed','url',None,None,{}, {})
                else:
                    c.capture_http_session(self.run,self.root,'fixed','url',None,None,{}, {})
                    self.assertEqual((self.run/'http-server').read_bytes(),b'binary')
                    self.assertGreaterEqual(kill.call_count,1)
                self.assertEqual(command.call_args_list[0].kwargs['compiler_target'],self.target)

    def test_http_observations_preserve_auth_contract_and_health(self):
        import http_auth, http_scenarios, http_contract
        for adapter in (None,{'login':[{'fixture':True}],'headers':{}}):
            tls=MagicMock()
            bridge=MagicMock();bridge.seal.return_value=([{'sealed':True}],{'valid':True})
            (self.root/'api/capture-scenarios.json').write_text('[]')
            with patch.object(c,'prepare_http_fixture'),patch.object(http_auth,'bootstrap') as bootstrap,patch.object(http_scenarios,'capture',return_value=[{'observation':True}]),patch.object(http_contract,'ContractCapture',return_value=bridge),patch.object(c,'capture_health'):
                result=c.capture_http_observations(self.run,self.root,'fixed',self.target/'debug/codexsymphony-server',{},'127.0.0.1:1',adapter,tls,{})
                self.assertTrue(result)
                self.assertEqual(bootstrap.call_count,int(adapter is not None))
                if adapter:bridge.observe.assert_called_once()

    def response(self,status):
        response=io.BytesIO(b'{}');response.status=status;response.headers={'Content-Type':'application/json'}
        return response

    def test_health_checks_stop_database_and_record_expected_unavailable(self):
        for tls in (None,SimpleNamespace(origin='https://127.0.0.1:1',context=None)):
            opener=MagicMock();opener.open.side_effect=[self.response(200),urllib.error.HTTPError('http://fixture',503,'Unavailable',{'Content-Type':'application/json'},io.BytesIO(b'{}'))]
            observations=[]
            with patch.object(c.urllib.request,'build_opener',return_value=opener),patch.object(c.subprocess,'run') as stop:
                c.capture_health('127.0.0.1:1','fixed',tls,observations)
                stop.assert_called_once()
            self.assertEqual([x['status'] for x in observations],[200,503])
        with patch.object(c.urllib.request,'build_opener') as opener:
            opener.return_value.open.return_value=self.response(500)
            with self.assertRaises(RuntimeError):c.capture_health('127.0.0.1:1','fixed',None,[])

    def frontend(self):
        directory=self.run/'tmp/codexsymphony-ts-risk-fixture';directory.mkdir(parents=True)
        request={'parameters':{'exclude':['src/one.spec.ts'],'receipt':{'coverage_root':'/original','inputs':{'src/one.ts':'hash'},'pipeline':{'files':{'package.json':'hash'},'tools':{}}}}}
        (directory/'collector-bundle.json').write_text(json.dumps({'request':request}))
        (directory/'coverage.json').write_text(json.dumps({'/original/src/one.ts':{'path':'/original/src/one.ts'}}))
        return directory

    def test_frontend_binding_preserves_sources_and_rejects_wrong_prefix(self):
        directory=self.frontend()
        with patch.object(c,'node',side_effect=[{'subjects':[{'id':'one'}],'sources':[{'path':'web/angular/src/one.ts','sha256':'hash'}]},'binding']):
            request,discovery=c.bind_frontend(self.run,self.root,self.run,self.run,{'run':'fixture'})
        self.assertEqual(request['parameters']['exclude'],['web/angular/src/one.spec.ts'])
        self.assertEqual(request['parameters']['receipt']['request'],'binding')
        self.assertIn('/harness-capture/web/angular/src/one.ts',json.loads((self.run/'frontend-coverage.json').read_text()))
        (directory/'coverage.json').write_text('{"/wrong/file":{}}')
        with self.assertRaises(ValueError):c.bind_frontend(self.run,self.root,self.run,self.run,{})

    def test_contract_binding_keeps_optional_fixture_auth_and_native_identity(self):
        names=['api/openapi.json','web/angular/src/app/health.ts','web/angular/src/app/health-response.ts','apps/server/src/lib.rs','apps/server/src/main.rs','Cargo.toml','Cargo.lock','apps/server/Cargo.toml','api/capture-scenarios.json','api/capture-fixture.sql','api/capture-auth.json']
        for name in names:
            p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('fixture')
        (self.run/'http-auth-validation.json').write_text('{"valid":true}')
        with patch.object(c,'node',return_value={'subjects':['contract']}):
            q=c.bind_contract(self.run,self.root,self.run,self.run,{}, {'commit':'baseline'}, [],'binary',{'parameters':{'exclude':[]}}, {'sources':[{'path':'client','sha256':'hash'}]})
        receipt=q['parameters']['receipt']
        self.assertEqual(set(receipt['inputs']),set(names))
        self.assertEqual(receipt['binary_sha256'],'binary')
        self.assertTrue(receipt['auth_validation']['valid'])

    def test_complete_binding_checks_source_inventory_and_records_series(self):
        backend=self.run/'probes/backend';backend.mkdir(parents=True)
        (backend/'bundle.json').write_text(json.dumps({'request':{'parameters':{'receipt':{'sources':{'source':'hash'}}}}}))
        plugin=SimpleNamespace(inventory=lambda q:{'source':'hash'},series=lambda q:{'id':'backend'})
        with patch.dict(sys.modules,{'plugin':plugin}),patch.object(c,'capture_producers',return_value=([],'binary')),patch.object(c,'bind_frontend',return_value=({},{})),patch.object(c,'bind_contract',return_value={}),patch.object(c,'node',return_value={'id':'series'}):
            requests,identities=c.captures(self.run,self.root,self.root,{'run':'bound'}, {})
            self.assertEqual(requests['backend']['parameters']['receipt']['context'],{'run':'bound'})
            self.assertEqual(identities['backend'],{'id':'backend'})
            plugin.inventory=lambda q:{}
            with self.assertRaises(ValueError):c.captures(self.run,self.root,self.root,{}, {})


if __name__=='__main__':unittest.main()

"""Real TLS capture -> pinned rc.5 measure -> sentinel-scanned retention."""
import copy
import hashlib
import http.server
import json
import os
import shutil
import secrets
import tempfile
import threading
import unittest
from pathlib import Path
from http_contract import ContractCapture
from http_scenarios import capture
from http_tls import TLSCapture
from test_http_auth import Api


def obj(properties):
    return {'type':'object','additionalProperties':False,'required':list(properties),'properties':properties}


def response(schema):
    return {'description':'probe','content':{'application/json':{'schema':schema}}}


def contract():
    proof=obj({'csrf':{'type':'string','minLength':20}})
    return {'openapi':'3.0.3','info':{'title':'Auth bridge probe','version':'1'},'paths':{
        '/api/csrf':{'get':{'operationId':'getCsrf','responses':{'200':response(proof)}}},
        '/api/login':{'post':{'operationId':'login','requestBody':{'required':True,'content':{'application/json':{'schema':obj({'username':{'type':'string'},'password':{'type':'string','minLength':32}})}}},'responses':{'200':response(proof)}}},
        '/api/write':{'post':{'operationId':'write','responses':{'200':response(obj({'written':{'type':'boolean'}})), '403':response(obj({'written':{'type':'boolean'}}))}}}}}


class AlternateProofApi(Api):
    def send(self, status, body, cookie=None):
        if 'csrf' in body:
            body = {'proof':body['csrf'], 'role':'admin'}
        super().send(status, body, cookie)


class ContractBridgeTests(unittest.TestCase):
    def test_verified_tls_full_measurement_and_retained_sentinel_scan(self):
        with tempfile.TemporaryDirectory() as directory, TLSCapture() as tls:
            root=Path(directory)
            variables={'username':'probe-'+secrets.token_hex(8),'password':secrets.token_urlsafe(32)}
            account=root/'account.json'
            account.write_text(json.dumps({'username':variables['username'],'digest':hashlib.sha256(variables['password'].encode()).hexdigest()}))
            api=http.server.ThreadingHTTPServer(('127.0.0.1',0),AlternateProofApi)
            api.account=account;api.origin=tls.origin
            api.proof=secrets.token_hex(32);api.session_proof=secrets.token_hex(32);api.session=secrets.token_hex(32)
            thread=threading.Thread(target=api.serve_forever,daemon=True);thread.start()
            address=f'127.0.0.1:{api.server_port}';tls.start(address)
            bridge=ContractCapture(variables);spec=contract()
            proof_schema=obj({'proof':{'type':'string','minLength':20},'role':{'type':'string','enum':['admin']}})
            for path,method in [('/api/csrf','get'),('/api/login','post')]:
                spec['paths'][path][method]['responses']['200']=response(proof_schema)
            setup=[{'id':'csrf','method':'GET','path':'/api/csrf','status':200,'record':False},
                   {'id':'login','method':'POST','path':'/api/login','status':200,'record':False,
                    'headers':{'x-codexsymphony-csrf':{'$response':'csrf#/proof'}},
                    'body':{'username':{'$capture':'username'},'password':{'$capture':'password'}}}]
            try:
                # Separate unauthenticated session produces the actual negative variant.
                capture(address,spec,[{'id':'denied','method':'POST','path':'/api/write','status':403,'body':{}}],tls=tls,observer=bridge)
                rows=capture(address,spec,setup+[
                    {'id':'csrfRecorded','method':'GET','path':'/api/csrf','status':200},
                    {'id':'write','method':'POST','path':'/api/write','status':200,'body':{}}],
                    tls=tls,variables=variables,setup_count=2,
                    default_headers={'x-codexsymphony-csrf':{'$response':'login#/proof'}},observer=bridge)
                self.assertEqual(rows, [])
                safe,receipt=bridge.seal(spec)
                self.assertEqual(set(receipt['variants']),{'GET /api/csrf:200','POST /api/login:200','POST /api/write:200','POST /api/write:403'})
                self.assertTrue(receipt['validation']['metrics']['contract.compatible']['value'])
                retained=root/'retained';retained.mkdir()
                (retained/'observations.json').write_text(json.dumps(safe,indent=2)+'\n')
                (retained/'validation.json').write_text(json.dumps(receipt))
                sentinels=[variables['password'],api.session,api.proof,api.session_proof]
                self.assertTrue(all(s not in p.read_text() for p in retained.iterdir() for s in sentinels))
                if os.environ.get('HTTP_CAPTURE_SMOKE_OUTPUT'):
                    destination=Path(os.environ['HTTP_CAPTURE_SMOKE_OUTPUT'])
                    destination.mkdir(parents=True,exist_ok=True)
                    for path in retained.iterdir(): shutil.copyfile(path,destination/path.name)
                    (destination/'sentinel-scan.json').write_text(json.dumps({'password_session_csrf_matches':0,'declared_variants':4,'login_success_observed':True}))
                self.assertEqual(receipt['observations_sha256'],hashlib.sha256((retained/'observations.json').read_bytes()).hexdigest())
            finally:
                api.shutdown();api.server_close();thread.join()

    def test_business_authorizations_and_setup_enums_are_not_credentials(self):
        kind = {'type':'string','enum':['code_change','validation_only']}
        document = obj({'children':{'type':'array','items':obj({'kind':kind})}})
        business = obj({'document':document,
                        'authorizations':{'type':'array','items':obj({'snapshot':document})},
                        'authorization':obj({'decision':{'type':'string','enum':['approved']},
                                             'csrfToken':{'type':'string'}}),
                        'session':obj({'state':{'type':'string','enum':['active']}}),
                        'secret_review':{'type':'string','enum':['code_change']},
                        'note':{'type':'string'}})
        spec = {'openapi':'3.0.3','info':{'title':'business regression','version':'1'},
                'paths':{'/api/group':{'get':{'operationId':'group','responses':{'200':response(business)}}}}}
        password, cookie, proof = [secrets.token_urlsafe(32) for _ in range(3)]
        draft = {'children':[{'kind':'code_change'},{'kind':'validation_only'}]}
        body = {'document':draft,'authorizations':[{'snapshot':copy.deepcopy(draft)}],
                'authorization':{'decision':'approved','csrfToken':proof},
                'session':{'state':'active'},'secret_review':'code_change',
                'note':'echo '+password+' '+cookie+' '+proof}
        bridge = ContractCapture({'password':password})
        bridge.observe({'method':'GET','path':'/api/group','status':200,
                        'content_type':'application/json','body':body}, setup=True,cookies=[cookie])
        self.assertTrue(bridge.secrets == {password,cookie,proof})
        safe, _ = bridge.seal(spec)
        self.assertEqual(safe[0]['body']['document'],draft)
        self.assertEqual(safe[0]['body']['authorizations'],body['authorizations'])
        self.assertTrue(all(secret not in json.dumps(safe) for secret in (password,cookie,proof)))

    def observations(self):
        return [
            {'method':'GET','path':'/api/csrf','status':200,'content_type':'application/json','body':{'csrf':'a'*40}},
            {'method':'POST','path':'/api/login','status':200,'content_type':'application/json','body':{'csrf':'b'*40},'request_body':{'username':'probe','password':'p'*40}},
            *[{'method':'POST','path':'/api/write','status':status,'content_type':'application/json','body':{'written':status==200}} for status in (200,403)]]

    def test_rejects_raw_schema_missing_coverage_duplicates_and_mask_constraints(self):
        base=self.observations()
        cases=[]
        short=copy.deepcopy(base);short[1]['request_body']['password']='bad';cases.append((contract(),short))
        wrong=copy.deepcopy(base);wrong[1]['body']['csrf']=42;cases.append((contract(),wrong))
        cases.append((contract(),base[:-1]))
        cases.append((contract(),base+[base[-1]]))
        constrained=contract();constrained['paths']['/api/login']['post']['requestBody']['content']['application/json']['schema']['properties']['password']['pattern']='^p+$'
        cases.append((constrained,base))
        for spec,rows in cases:
            with self.subTest(case=len(rows)):
                bridge=ContractCapture({'password':'p'*40})
                for row in rows: bridge.observe(row)
                with self.assertRaisesRegex(ValueError,'credential-safe contract validation failed'):
                    bridge.seal(spec)


if __name__=='__main__':unittest.main(verbosity=2)

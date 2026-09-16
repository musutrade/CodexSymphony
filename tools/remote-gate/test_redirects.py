"""Real urllib redirect processing over local HTTP; credentials are synthetic."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import unittest
from unittest.mock import patch
import urllib.error
import github


class CredentialRedirects(unittest.TestCase):
    def setUp(self):
        self.seen=[]
        self.status=200
        self.location='/destination'
        test=self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_GET(self): self.respond()
            def do_POST(self): self.respond()
            def respond(self):
                body=self.rfile.read(int(self.headers.get('Content-Length','0')))
                test.seen.append((self.server.server_port,self.path,self.command,self.headers.get('Authorization'),body))
                status=test.status if self.path=='/request' else 200
                self.send_response(status)
                if 300<=status<400:self.send_header('Location',test.location)
                self.send_header('Content-Type','application/json')
                self.end_headers();self.wfile.write(b'{"ok":true}')

        self.servers=[ThreadingHTTPServer(('127.0.0.1',0),Handler) for _ in range(2)]
        for server in self.servers:
            threading.Thread(target=server.serve_forever,daemon=True).start()
        self.origin=f'http://127.0.0.1:{self.servers[0].server_port}'
        self.patcher=patch.object(github,'API_ROOT',self.origin)
        self.patcher.start()

    def tearDown(self):
        self.patcher.stop()
        for server in self.servers:server.shutdown();server.server_close()

    def test_direct_authenticated_requests_still_work(self):
        self.assertEqual(github.request('/request','synthetic-token','POST',{'fixture':1}),{'ok':True})
        self.assertEqual(len(self.seen),1)
        self.assertEqual(self.seen[0][2:4],('POST','Bearer synthetic-token'))
        self.assertEqual(json.loads(self.seen[0][4]),{'fixture':1})

    def test_redirects_never_reach_same_or_cross_origin_sink_or_replay_writes(self):
        for method in ('GET','POST'):
            for status in (301,302,303,307,308):
                for location in ('/destination',f'http://127.0.0.1:{self.servers[1].server_port}/destination',
                                 f'//127.0.0.1:{self.servers[1].server_port}/destination'):
                    with self.subTest(method=method,status=status,location=location):
                        self.seen.clear();self.status=status;self.location=location
                        with self.assertRaises(urllib.error.HTTPError) as error:
                            github.request('/request','synthetic-token',method)
                        self.assertEqual(error.exception.code,status)
                        error.exception.close()
                        self.assertEqual(len(self.seen),1)
                        self.assertEqual(self.seen[0][0],self.servers[0].server_port)

    def test_https_downgrade_is_rejected_by_same_handler(self):
        # No TLS connection is made: exercise redirect handling for an HTTPS
        # origin and an HTTP destination, while the matrix tests real transport.
        request=github.urllib.request.Request('https://api.github.com/request',headers={'Authorization':'Bearer synthetic-token'})
        with self.assertRaises(urllib.error.HTTPError) as error:
            github.RejectRedirects().redirect_request(request,None,302,'Found',{},self.origin+'/destination')
        error.exception.close()
        self.assertEqual(self.seen,[])

    def test_non_success_is_not_retried(self):
        self.status=403
        with self.assertRaises(urllib.error.HTTPError) as error:
            github.request('/request','synthetic-token')
        error.exception.close()
        self.assertEqual(len(self.seen),1)

    def test_invalid_origin_paths_fail_before_transport(self):
        for path in ('https://evil.invalid/','//evil.invalid/','@evil.invalid/','/path#fragment','/path\nnext'):
            with self.subTest(path=path),self.assertRaises(ValueError):github.request(path,'synthetic-token')
        self.assertEqual(self.seen,[])


if __name__=='__main__':unittest.main()

"""Real TLS/session/CSRF transport and isolated stdin bootstrap capability smoke.

The probe API is deliberately a test fixture, not product AAuth acceptance.
"""
import hashlib
import http.server
import json
import secrets
import ssl
import tempfile
import threading
import unittest
import urllib.request
from pathlib import Path
from http_auth import bootstrap, load_adapter, prepare
from http_scenarios import capture
from http_tls import TLSCapture


class Api(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass

    def send(self, status, body, cookie=None):
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        if cookie: self.send_header('Set-Cookie', cookie)
        self.end_headers()
        self.wfile.write(json.dumps(body).encode())

    def do_GET(self):
        if self.path == '/api/csrf':
            self.send(200, {'csrf': self.server.proof}); return
        if self.path == '/api/redirect':
            self.send_response(302); self.send_header('Location', 'https://example.com/'); self.end_headers(); return
        self.send(404, {})

    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))))
        proof = self.headers.get('x-codexsymphony-csrf')
        if self.path == '/api/login':
            account = json.loads(self.server.account.read_text())
            valid = (proof == self.server.proof and data.get('username') == account['username']
                     and hashlib.sha256(data.get('password', '').encode()).hexdigest() == account['digest'])
            if valid:
                self.send(200, {'csrf': self.server.session_proof},
                          'capture_session='+self.server.session+'; HttpOnly; Secure; SameSite=Lax; Path=/')
            else: self.send(401, {'error': 'login failed'})
            return
        valid = (self.headers.get('Cookie') == 'capture_session='+self.server.session
                 and proof == self.server.session_proof
                 and self.headers.get('Origin') == self.server.origin
                 and self.headers.get('X-Forwarded-Proto') == 'https')
        self.send(200 if valid else 403, {'written': valid})


class AuthCaptureTests(unittest.TestCase):
    def test_tls_cookie_response_csrf_and_restricted_bootstrap(self):
        with tempfile.TemporaryDirectory() as directory, TLSCapture() as tls:
            root = Path(directory); repo = root/'repo'; repo.mkdir(); (repo/'api').mkdir()
            (repo/'environment.lock.json').write_bytes((Path(__file__).resolve().parents[2]/'environment.lock.json').read_bytes())
            run = root/'run'; run.mkdir()
            secret = root/'control-plane-sentinel'; secret.write_text('must-not-read')
            binary = repo/'admin'
            binary.write_text('''#!/usr/bin/python3
import hashlib,json,os,sys
from pathlib import Path
assert sys.argv[1:] == ['auth','init','--stdin-json']
assert 'GITHUB_TOKEN' not in os.environ and 'DATABASE_URL' not in os.environ
assert not Path('''+repr(str(secret))+''').exists()
a=json.load(sys.stdin)
assert set(a)=={'username','password'}
p=Path('/tmp/account.json');p.write_text(json.dumps({'username':a['username'],'digest':hashlib.sha256(a['password'].encode()).hexdigest()}));p.chmod(0o600)
'''); binary.chmod(0o700)
            adapter = {'schema': 'codexsymphony-http-auth/v1', 'config': {'public_origin': {'$capture': 'origin'}},
                       'bootstrap_args': ['auth', 'init', '--stdin-json'],
                       'bootstrap_input': {'username': {'$capture': 'username'}, 'password': {'$capture': 'password'}},
                       'login': [
                           {'id': 'csrf', 'method': 'GET', 'path': '/api/csrf', 'status': 200, 'record': False},
                           {'id': 'login', 'method': 'POST', 'path': '/api/login', 'status': 200, 'record': False,
                            'headers': {'x-codexsymphony-csrf': {'$response': 'csrf#/csrf'}},
                            'body': {'username': {'$capture': 'username'}, 'password': {'$capture': 'password'}}}],
                       'headers': {'x-codexsymphony-csrf': {'$response': 'login#/csrf'}}}
            path = repo/'api/capture-auth.json'; path.write_text(json.dumps(adapter))
            self.assertEqual(load_adapter(repo), adapter)
            variables, env = prepare(adapter, run, tls.origin)
            bootstrap(adapter, variables, binary=binary, run=run, repository=repo,
                      plugins=repo, environment=env)
            account = run/'tmp/account.json'
            self.assertEqual(account.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(variables['password'], account.read_text())
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Api)
            server.account = account; server.origin = tls.origin
            server.proof = secrets.token_urlsafe(); server.session_proof = secrets.token_urlsafe(); server.session = secrets.token_urlsafe()
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            address = f'127.0.0.1:{server.server_port}'; tls.start(address)
            spec = {'paths': {'/api/csrf': {'get': {'responses': {'200': {}}}},
                              '/api/login': {'post': {'responses': {'200': {}}}},
                              '/api/write': {'post': {'responses': {'200': {}, '403': {}}}},
                              '/api/redirect': {'get': {'responses': {'302': {}}}}}}
            write = {'id': 'write', 'method': 'POST', 'path': '/api/write', 'status': 200, 'body': {}}
            try:
                observed = capture(address, spec, adapter['login']+[write], tls=tls, variables=variables,
                                   default_headers=adapter['headers'], setup_count=2)
                self.assertEqual(len(observed), 1)
                self.assertEqual(observed[0]['body'], {'written': True})
                self.assertNotIn(variables['password'], json.dumps(observed))
                self.assertNotIn(server.session, json.dumps(observed))
                # A fresh capture gets no prior cookies; a stale CSRF proof fails.
                no_login = dict(write, status=403, headers={'x-codexsymphony-csrf': server.session_proof})
                self.assertFalse(capture(address, spec, [no_login], tls=tls)[0]['body']['written'])
                stale = dict(write, status=403, headers={'x-codexsymphony-csrf': '1'})
                self.assertFalse(capture(address, spec, adapter['login']+[stale], tls=tls, variables=variables)[0]['body']['written'])
                # Certificate validation is real, not CERT_NONE / ignore HTTPS errors.
                with self.assertRaises(urllib.error.URLError):
                    urllib.request.build_opener(urllib.request.ProxyHandler({})).open(tls.origin+'/api/csrf',timeout=5)
                for headers in [{'Cookie': 'injected'}, {'Authorization': 'injected'},
                                {'x-codexsymphony-csrf': 'x\r\ny'}]:
                    with self.assertRaises(ValueError): capture(address, spec, [dict(write, headers=headers)], tls=tls)
                with self.assertRaises(ValueError):
                    capture(address, spec, [{'id':'redirect','method':'GET','path':'/api/redirect','status':302}], tls=tls)
            finally:
                server.shutdown(); server.server_close(); thread.join()
            for edit in [{'bootstrap_args':['/bin/sh']}, {'bootstrap_args':['auth','--file=/secrets']},
                         {'login':[dict(adapter['login'][0],record=True)]}, {'headers':{'Cookie':'injected'}}]:
                path.write_text(json.dumps(adapter | edit))
                with self.assertRaises(ValueError): load_adapter(repo)
            path.unlink(); path.symlink_to(secret)
            with self.assertRaises(ValueError): load_adapter(repo)


if __name__ == '__main__': unittest.main(verbosity=2)

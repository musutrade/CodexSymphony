"""Disposable product HTTPS browser fixture; run with the supplied DB wrapper.
No production ingress, credentials, database, or trust store is modified.
"""
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import secrets
import signal
import ssl
import subprocess
import tempfile
import threading
import urllib.parse

from auth_contract_acceptance import ROOT, sql, wait_http_address, TLSCapture
from http_tls import Relay


class BrowserRelay(Relay):
    def do_GET(self):
        if self.path.startswith('/api/'):
            return self.relay()
        relative = urllib.parse.urlsplit(self.path).path.lstrip('/')
        root = ROOT / 'web/angular/dist/codexsymphony-web/browser'
        path = (root / relative).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            path = root / 'index.html'
        import mimetypes
        data = path.read_bytes()
        self.send_response(200)
        self.send_header('Content-Type', mimetypes.guess_type(path)[0] or 'application/octet-stream')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def main():
    base = os.environ['TEST_DATABASE_URL']
    schema = 'auth_browser_' + secrets.token_hex(8)
    sql(base, 'CREATE SCHEMA ' + schema)
    parts = urllib.parse.urlsplit(base)
    query = urllib.parse.parse_qsl(parts.query) + [('options', '-csearch_path=' + schema)]
    url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
    binary = (ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'debug/codexsymphony-server').resolve()
    try:
        with tempfile.TemporaryDirectory(prefix='gh71-browser-') as directory, TLSCapture() as tls:
            root = Path(directory)
            config = root / 'auth.json'
            config.write_text(json.dumps({'public_origin':tls.origin,'trusted_proxies':['127.0.0.1']}))
            config.chmod(0o600)
            env = dict(os.environ, DATABASE_URL=url, TEST_DATABASE_URL=url, AUTH_CONFIG=str(config), WEB_ORIGIN=tls.origin,
                       BIND_ADDRESS='127.0.0.1:0', EXECUTION_DIRECTORY=str(root/'execution'))
            for key in ['RUNTIME_CONFIG','STORAGE_CONFIG','GITHUB_CONFIG']:
                env.pop(key, None)
            server = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                with open(os.devnull, 'wb') as discard:
                    address = wait_http_address(server, discard)
                tls.server.RequestHandlerClass = BrowserRelay
                tls.start(address)
                cert = Path(tls.directory.name)/'cert.pem'
                public = subprocess.check_output(['openssl','x509','-in',str(cert),'-pubkey','-noout'])
                der = subprocess.check_output(['openssl','pkey','-pubin','-outform','DER'], input=public)
                env.update(E2E_HTTPS_ORIGIN=tls.origin, E2E_AUTH_BINARY=str(binary),
                           NODE_EXTRA_CA_CERTS=str(cert),
                           E2E_TLS_SPKI=base64.b64encode(hashlib.sha256(der).digest()).decode())
                result = subprocess.run(['npm','run','test:e2e'], cwd=ROOT/'web/angular', env=env)
                return result.returncode
            finally:
                server.send_signal(signal.SIGINT)
                try: server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill(); server.wait()
    finally:
        sql(base, 'DROP SCHEMA ' + schema + ' CASCADE')


if __name__ == '__main__':
    raise SystemExit(main())

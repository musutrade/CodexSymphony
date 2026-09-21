"""Local real HTTPS contract acceptance using the supplied credential-safe observer.
Run through /opt/symphony-env/run.py. Only the disposable TEST_DATABASE_URL is used.
No signed Gate result is produced. Raw credentials/responses stay in memory.
"""
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import socketserver
import select
import threading
import subprocess
import sys
import tempfile
import urllib.parse

sys.path.insert(0, '/opt/symphony-env/http-capture')
from http_auth import load_adapter
from http_contract import ContractCapture
from http_scenarios import capture, capture_values
from http_tls import TLSCapture
from capture import wait_http_address, HTTP

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/gh71-product-contract'


def sql(url, query=None, file=None):
    env = dict(os.environ, PGDATABASE=url)
    args = ['psql', '-X', '-d', url, '-v', 'ON_ERROR_STOP=1']
    args += ['-f', str(file)] if file else ['-c', query]
    subprocess.run(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)


class DatabaseRelay(socketserver.ThreadingTCPServer):
    """Owned test-only connection fault; never stops or changes the supplied DB."""
    daemon_threads = True
    def __init__(self, upstream):
        self.upstream = upstream
        self.connections = []
        super().__init__(('127.0.0.1', 0), DatabaseConnection)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()
    def stop(self):
        self.shutdown()
        self.server_close()
        for connection in self.connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
                connection.close()
            except OSError:
                pass
        self.thread.join()

class DatabaseConnection(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            with socket.create_connection(self.server.upstream) as upstream:
                pair = [self.request, upstream]
                self.server.connections.extend(pair)
                while True:
                    for source in select.select(pair, [], [], 1)[0]:
                        data = source.recv(65536)
                        if not data:
                            return
                        pair[1 - pair.index(source)].sendall(data)
        except (OSError, ValueError):
            pass


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    base = os.environ['TEST_DATABASE_URL']
    schema = 'auth_capture_' + secrets.token_hex(8)
    sql(base, 'CREATE SCHEMA ' + schema)
    parts = urllib.parse.urlsplit(base)
    query = urllib.parse.parse_qsl(parts.query) + [('options', '-csearch_path=' + schema)]
    url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
    binary = ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'debug/codexsymphony-server'
    spec = json.loads((ROOT / 'api/openapi.json').read_text())
    adapter = load_adapter(ROOT)
    relay = DatabaseRelay((parts.hostname, parts.port or 5432))
    proxy_parts = urllib.parse.urlsplit(url)
    server_url = urllib.parse.urlunsplit(proxy_parts._replace(netloc=proxy_parts.netloc.rsplit('@', 1)[0] + '@127.0.0.1:' + str(relay.server_address[1])))
    try:
        with tempfile.TemporaryDirectory(prefix='gh71-auth-') as directory, TLSCapture() as tls:
            root = Path(directory)
            variables = {'origin': tls.origin, 'username': 'capture-' + secrets.token_hex(8), 'password': secrets.token_urlsafe(32)}
            config = root / 'auth.json'
            config.write_text(json.dumps(capture_values(adapter['config'], variables)))
            config.chmod(0o600)
            env = dict(os.environ, DATABASE_URL=server_url, AUTH_CONFIG=str(config), WEB_ORIGIN=tls.origin, BIND_ADDRESS='127.0.0.1:0', EXECUTION_DIRECTORY=str(root / 'execution'))
            for key in ['RUNTIME_CONFIG', 'STORAGE_CONFIG', 'GITHUB_CONFIG']:
                env.pop(key, None)
            server = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                with open(os.devnull, 'wb') as discard:
                    address = wait_http_address(server, discard)
                tls.start(address)
                sql(url, file=ROOT / 'api/capture-fixture.sql')
                subprocess.run([str(binary), *adapter['bootstrap_args']], env=env,
                               input=json.dumps(capture_values(adapter['bootstrap_input'], variables)).encode(),
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
                bridge = ContractCapture(variables)
                scenarios = adapter['login'] + json.loads((ROOT / 'api/capture-scenarios.json').read_text())
                capture(address, spec, scenarios, tls=tls, variables=variables, default_headers=adapter['headers'], setup_count=len(adapter['login']), observer=bridge)
                capture(address, spec, [{'id':'health','method':'GET','path':'/api/health','status':200}], tls=tls, observer=bridge)
                relay.stop()
                capture(address, spec, [{'id':'health-unavailable','method':'GET','path':'/api/health','status':503}], tls=tls, observer=bridge)
                safe, receipt = bridge.seal(spec)
                (OUT / 'observations.json').write_text(json.dumps(safe, indent=2) + '\n')
                (OUT / 'validation.json').write_text(json.dumps(receipt, indent=2) + '\n')
                subprocess.run(['node', str(ROOT/'tools/auth_contract_measure.cjs'), str(ROOT), str(HTTP)], check=True)
                print(json.dumps({'validated_variants':len(safe), 'raw_credentials_retained':False}))
                server.send_signal(signal.SIGINT)
                server.wait(timeout=10)
            finally:
                if server.poll() is None:
                    server.kill()
                    server.wait()
    finally:
        if relay.thread.is_alive():
            relay.stop()
        sql(base, 'DROP SCHEMA ' + schema + ' CASCADE')


if __name__ == '__main__':
    main()

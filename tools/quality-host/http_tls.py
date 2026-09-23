"""Ephemeral verified TLS relay to one capture-owned loopback API, no redirects."""
import http.client
import http.server
import re
import ssl
import subprocess
import tempfile
import threading
from pathlib import Path

LIMIT = 1024 * 1024
HOP = {'connection', 'keep-alive', 'proxy-authenticate', 'proxy-authorization',
       'te', 'trailer', 'transfer-encoding', 'upgrade'}


class Relay(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def relay(self):
        if (not self.path.startswith('/api/') or any(c in self.path for c in ['\\', '#'])
                or self.headers.get('Transfer-Encoding') or len(self.headers.get_all('Content-Length', [])) > 1):
            self.send_error(400); return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 <= length <= LIMIT:
                raise ValueError('request too large')
            data = self.rfile.read(length)
            headers = {k: v for k, v in self.headers.items()
                       if k.lower() not in HOP | {'host', 'forwarded', 'x-forwarded-for', 'x-forwarded-proto', 'x-forwarded-host', 'content-length'}}
            headers.update({'Host': self.server.backend, 'X-Forwarded-For': '127.0.0.1',
                            'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': self.headers.get('Host', '')})
            connection = http.client.HTTPConnection(self.server.backend, timeout=8)
            try:
                connection.request(self.command, self.path, body=data, headers=headers)
                response = connection.getresponse()
                body = response.read(LIMIT + 1)
                if len(body) > LIMIT:
                    raise ValueError('response too large')
                self.send_response(response.status)
                for name, value in response.getheaders():
                    if name.lower() not in HOP | {'content-length', 'server', 'date'}:
                        self.send_header(name, value)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            finally:
                connection.close()
        except (ValueError, OSError, http.client.HTTPException):
            self.send_error(502, 'capture relay failed')

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = relay


class TLSCapture:
    def __enter__(self):
        self.directory = tempfile.TemporaryDirectory(prefix='trusted-capture-tls-')
        root = Path(self.directory.name)
        cert, key = root/'cert.pem', root/'key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
                        '-keyout', str(key), '-out', str(cert), '-days', '1',
                        '-subj', '/CN=127.0.0.1', '-addext', 'subjectAltName=IP:127.0.0.1'],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=15)
        key.chmod(0o600)
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Relay)
        self.server.backend = None
        self.server.timeout = 8
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(cert, key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.context = ssl.create_default_context(cafile=str(cert))
        self.origin = f'https://127.0.0.1:{self.server.server_port}'
        self.thread = None
        return self

    def start(self, address):
        if not re.fullmatch(r'127\.0\.0\.1:[1-9]\d{0,4}', address) or int(address.split(':')[1]) > 65535:
            raise ValueError('capture-owned loopback backend required')
        self.server.backend = address
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def __exit__(self, *args):
        if self.thread:
            self.server.shutdown()
            self.thread.join(timeout=10)
        self.server.server_close()
        self.directory.cleanup()

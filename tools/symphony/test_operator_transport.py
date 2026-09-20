"""Exercise credential boundaries over real local HTTP with synthetic tokens."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import unittest
import urllib.error

import operator_bridge as bridge


class OperatorTransportTests(unittest.TestCase):
    def setUp(self):
        self.seen = []
        self.status = 200
        self.location = '/destination'
        test = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.respond()

            def do_POST(self):
                self.respond()

            def respond(self):
                body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                test.seen.append((self.path, self.command, self.headers.get('Authorization'), body))
                status = test.status if self.path == '/request' else 200
                self.send_response(status)
                if 300 <= status < 400:
                    self.send_header('Location', test.location)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{"ok": true}')

        self.servers = [ThreadingHTTPServer(('127.0.0.1', 0), Handler) for _ in range(2)]
        for server in self.servers:
            threading.Thread(target=server.serve_forever, daemon=True).start()
            self.addCleanup(server.server_close)
            self.addCleanup(server.shutdown)
        self.origin = f'http://127.0.0.1:{self.servers[0].server_port}'

    def test_direct_recovery_preserves_authentication_and_body(self):
        body = {'action': 'resume', 'revision': 1}
        self.assertEqual(bridge.api(self.origin, '/request', 'synthetic-token', body), {'ok': True})
        self.assertEqual(self.seen[0][:3], ('/request', 'POST', 'Bearer synthetic-token'))
        self.assertEqual(json.loads(self.seen[0][3]), body)

    def test_redirects_never_forward_credentials_or_replay_recovery(self):
        sink = f'127.0.0.1:{self.servers[1].server_port}/destination'
        for body in (None, {'action': 'resume'}):
            for status in (301, 302, 303, 307, 308):
                for location in ('/destination', 'http://' + sink, '//' + sink):
                    with self.subTest(body=body, status=status, location=location):
                        self.seen.clear()
                        self.status, self.location = status, location
                        with self.assertRaises(urllib.error.HTTPError) as error:
                            bridge.api(self.origin, '/request', 'synthetic-token', body)
                        self.assertEqual(error.exception.code, status)
                        error.exception.close()
                        self.assertEqual(len(self.seen), 1)
                        self.assertEqual(self.seen[0][0], '/request')


if __name__ == '__main__':
    unittest.main()

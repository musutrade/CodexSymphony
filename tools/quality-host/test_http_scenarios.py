"""The host talks to a real loopback server; scenarios never execute commands."""
import http.server
import json
import threading
import unittest
from http_scenarios import capture, resolve


class Api(http.server.BaseHTTPRequestHandler):
    calls = []

    def handle_write(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.calls.append((self.command, self.path, body, self.headers.get('x-codexsymphony-csrf')))
        status = 201 if self.command == 'POST' else 200
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({'id': 'one', 'version': len(self.calls), 'contract': body}).encode())

    do_POST = handle_write
    do_PATCH = handle_write

    def log_message(self, *args):
        pass


class ScenarioTests(unittest.TestCase):
    def test_real_post_patch_and_response_binding(self):
        Api.calls = []
        server = http.server.HTTPServer(('127.0.0.1', 0), Api)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            spec = {'paths': {'/api/requirements': {'post': {'responses': {'201': {}}}},
                              '/api/requirements/{id}': {'patch': {'responses': {'200': {}}}}}}
            steps = [{'id': 'create', 'method': 'POST', 'path': '/api/requirements', 'status': 201, 'body': {'title': 'A'}},
                     {'id': 'update', 'method': 'PATCH', 'path': '/api/requirements/{id}', 'status': 200,
                      'path_parameters': {'id': {'$response': 'create#/id'}}, 'body': {'title': 'B'}}]
            result = capture(f'127.0.0.1:{server.server_port}', spec, steps)
            self.assertEqual(result[1]['body']['version'], 2)
            self.assertEqual(result[1]['path'], '/api/requirements/one')
            self.assertEqual([item[3] for item in Api.calls], ['1', '1'])
            for change in [dict(steps[0], command='touch /tmp/unsafe'), dict(steps[0], path='http://elsewhere/api'),
                           dict(steps[0], headers={'Host': 'elsewhere'}), dict(steps[0], status=503)]:
                with self.assertRaises(ValueError):
                    capture(f'127.0.0.1:{server.server_port}', spec, [change])
            self.assertEqual(len(Api.calls), 2, 'rejected scenarios must not send requests')
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_external_listener_and_invalid_references_are_rejected(self):
        with self.assertRaises(ValueError):
            capture('example.com:80', {'paths': {}}, [])
        with self.assertRaises(ValueError):
            resolve({'$response': 'created#/id', 'extra': True}, {})


if __name__ == '__main__':
    unittest.main()

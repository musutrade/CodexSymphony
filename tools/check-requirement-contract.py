#!/usr/bin/env python3
"""Local business contract rehearsal against the disposable TEST_DATABASE_URL.

Run through /opt/symphony-env/run.py after cargo build and the database tests
(which clean their fixture). Does not sign Gate inputs. The trusted host owns
health 503 collection; this rehearsal covers business variants and health 200.
"""
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools/quality-host'))
from http_scenarios import capture  # noqa: E402


def main():
    url = os.environ['TEST_DATABASE_URL']
    output = ROOT / 'target/gh13-contract-observations.json'
    environment = dict(os.environ, DATABASE_URL=url, BIND_ADDRESS='127.0.0.1:0', RUST_LOG='info')
    server = subprocess.Popen([ROOT / 'target/debug/codexsymphony-server'], env=environment,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        selector = selectors.DefaultSelector()
        selector.register(server.stdout, selectors.EVENT_READ)
        address = None
        for _ in range(20):
            if selector.select(1):
                line = server.stdout.readline()
                if 'API listening at http://' in line:
                    address = line.split('API listening at http://', 1)[1].strip()
                    break
            if server.poll() is not None:
                raise RuntimeError('API exited before readiness')
        if not address:
            raise RuntimeError('API did not announce readiness')
        spec = json.loads((ROOT / 'api/openapi.json').read_text())
        scenarios = json.loads((ROOT / 'api/capture-scenarios.json').read_text())
        observations = capture(address, spec, scenarios)
        observations += health(address, 200)
        output.write_text(json.dumps(observations, indent=2) + '\n')
        print(f'Observed {len(observations)} declared operation/status variants: {output}')
    finally:
        server.terminate()
        server.wait(timeout=5)


def health(address, expected):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        response = opener.open('http://' + address + '/api/health', timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == expected, response.status
        return [dict(method='GET', path='/api/health', status=response.status,
                     content_type=response.headers['Content-Type'], body=json.load(response))]


if __name__ == '__main__':
    main()

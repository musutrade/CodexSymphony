#!/usr/bin/env python3
"""Host-owned, bounded external operations. Never load commands from an agent.

Registry grants bind issue, request, recovery revision, and the complete request
SHA256. Commands are host-owned absolute argv with a pinned executable digest.
A started receipt is durable before execution; ambiguous crash outcomes require
operator inspection instead of replaying a mutation. Completion retries only the
idempotent controller API. No journal edits or product ownership reset.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


class RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url, code, 'Operator API redirect rejected', headers, fp)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream, sort_keys=True)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def api(origin, route, token=None, body=None):
    request = urllib.request.Request(origin + route, data=None if body is None else json.dumps(body).encode())
    request.add_header('Content-Type', 'application/json')
    if token:
        request.add_header('Authorization', 'Bearer ' + token)
    # Host-local calls must never use inherited HTTP proxy settings.
    with urllib.request.build_opener(urllib.request.ProxyHandler({}), RejectRedirects()).open(request, timeout=20) as response:
        return json.load(response)


def matching_grant(entry, grants, profiles=()):
    request = entry.get('recovery', {}).get('external') or {}
    for grant in grants:
        if (grant.get('issue') == entry['issue_identifier']
                and grant.get('revision') == entry['recovery']['revision']
                and grant.get('request_sha256') == digest(request)
                and grant.get('operation') == request.get('operation')
                and grant.get('request_id') == request.get('request_id')):
            return grant
    for profile in profiles:
        if (profile.get('operation') == 'product.identity_preflight'
                and request.get('operation') == profile['operation']
                and request.get('repo') == profile.get('repo') == 'musutrade/CodexSymphony'
                and request.get('resume_condition') == profile.get('resume_condition')
                and isinstance(profile.get('resume_condition'), str)):
            return {**profile, 'issue': entry['issue_identifier'],
                    'revision': entry['recovery']['revision'], 'request_id': request['request_id'],
                    'request_sha256': digest(request)}
    return None


def run_operation(grant):
    argv = grant['argv']
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) for x in argv):
        raise ValueError('host argv must be a nonempty string list')
    executable = Path(argv[0])
    if not executable.is_absolute() or executable.is_symlink():
        raise ValueError('host executable must be an absolute regular file')
    if hashlib.sha256(executable.read_bytes()).hexdigest() != grant['executable_sha256']:
        raise ValueError('host executable changed since authorization')
    timeout = grant.get('timeout_seconds', 120)
    if not isinstance(timeout, int) or not 1 <= timeout <= 600:
        raise ValueError('operation timeout must be 1..600 seconds')
    # Output may contain service details; do not persist/log stdout or stderr.
    # The trusted operation is responsible for its own non-secret evidence.
    result = subprocess.run(argv, cwd='/', stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, timeout=timeout, check=False,
                            env={'PATH': '/usr/local/bin:/usr/bin:/bin', 'HOME': str(Path.home()),
                                 'XDG_RUNTIME_DIR': f'/run/user/{os.getuid()}'})
    return result.returncode


def process(entry, grant, receipts, complete, execute=run_operation):
    key = digest({'issue': entry['issue_identifier'], 'revision': entry['recovery']['revision'],
                  'external': entry['recovery']['external']})
    path = receipts / (key + '.json')
    receipt = json.loads(path.read_text()) if path.exists() else None
    if receipt is None:
        if grant is None:
            return 'awaiting_authorization'
        receipt = {'status': 'started', 'issue': entry['issue_identifier'], 'request_key': key,
                   'grant_sha256': digest(grant), 'started_at': int(time.time())}
        atomic_json(path, receipt)
        try:
            code = execute(grant)
            receipt.update(status='succeeded' if code == 0 else 'failed', exit_code=code)
        except (OSError, ValueError, subprocess.TimeoutExpired) as error:
            receipt.update(status='failed', error_type=type(error).__name__)
        receipt['completed_at'] = int(time.time())
        atomic_json(path, receipt)
    if receipt['status'] == 'succeeded':
        result = complete(entry['issue_identifier'], {
            'action': 'external_completed', 'request_id': 'host-' + key,
            'revision': entry['recovery']['revision'],
            'reason': 'Pinned authorized host operation completed', 'evidence': str(path)})
        receipt.update(status='acknowledged', recovery=result)
        atomic_json(path, receipt)
    return receipt['status']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--state', type=Path, default=Path.home() / '.local/share/codexsymphony/symphony/operator')
    parser.add_argument('--command', type=Path, help='explicit audited recovery JSON; requires --issue')
    parser.add_argument('--issue')
    args = parser.parse_args()
    os.umask(0o077)
    args.state.mkdir(parents=True, exist_ok=True)
    origin = 'http://127.0.0.1:4011'
    token = (args.state / 'token').read_text().strip()
    def complete(issue, command):
        if not issue.startswith('GH-') or not issue[3:].isdigit():
            raise ValueError('invalid issue identifier')
        return api(origin, '/api/v1/' + issue + '/recovery', token, command)
    if args.command:
        print(json.dumps(complete(args.issue or '', json.loads(args.command.read_text()))))
        return
    with (args.state / 'lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        registry = json.loads((args.state / 'grants.json').read_text())
        if registry.get('version') != 1 or not isinstance(registry.get('grants'), list):
            raise ValueError('invalid host grant registry')
        state = api(origin, '/api/v1/state')
        if state.get('error') or state.get('persistence_error'):
            raise RuntimeError('controller unavailable for durable external operations')
        for entry in state['waiting']:
            if entry['status'] == 'waiting_external':
                status = process(entry, matching_grant(entry, registry['grants'], registry.get('profiles', [])), args.state / 'receipts', complete)
                print(json.dumps({'issue': entry['issue_identifier'], 'status': status}))


if __name__ == '__main__':
    main()

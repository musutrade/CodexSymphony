#!/usr/bin/env python3
"""Host-only collection of disposable fixtures for merged, completed Issues.

Dry-run by default. Never infer completion from a missing workspace or age.
Product databases, shared images and nonterminal issue fixtures are excluded.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import time

BASE = Path.home() / '.local/share/codexsymphony'
LABEL = 'codexsymphony.fixture'
REPOSITORY = 'musutrade/CodexSymphony'


def run(*args):
    return subprocess.check_output(args, text=True, timeout=90).strip()


def inventory():
    result = {}
    for kind in ('container', 'volume', 'network'):
        args = ['docker', kind, 'ls', '--quiet']
        if kind == 'container':
            args.append('--all')
        identifiers = run(*args).splitlines()
        result[kind] = json.loads(run('docker', kind, 'inspect', *identifiers)) if identifiers else []
    return result


def completed(journal, issue):
    entry = journal['entries'].get(issue.removeprefix('GH-'), {})
    handoff = entry.get('handoff', {})
    return (entry.get('identifier') == issue and entry.get('phase') == 'done'
            and handoff.get('repo') == REPOSITORY
            and bool(re.fullmatch('[0-9a-f]{40}', handoff.get('merge_commit_sha', ''))))


def labels(kind, item):
    return (item.get('Config', {}).get('Labels') if kind == 'container' else item.get('Labels')) or {}


def plan(journal, resources):
    if journal.get('version') != 1 or not isinstance(journal.get('entries'), dict):
        raise ValueError('invalid host handoff journal')
    issues = set()
    for kind, items in resources.items():
        for item in items:
            label = labels(kind, item).get(LABEL, '')
            match = re.fullmatch(r'(GH-[1-9][0-9]*)(?:-(?:test|dev))?', label)
            if match:
                issues.add(match[1])
    plans = []
    for issue in sorted(issues):
        if not completed(journal, issue):
            plans.append({'issue': issue, 'eligible': False, 'reason': 'not completed with merge proof'})
            continue
        prefix = 'codexsymphony-' + issue.lower().replace('-', '')
        selected = {kind: [] for kind in resources}
        for kind, items in resources.items():
            for item in items:
                name = item['Name'].lstrip('/')
                label = labels(kind, item).get(LABEL)
                expected = ({prefix + '-test': issue + '-test', prefix + '-dev': issue + '-dev'}
                            if kind == 'container' else
                            {prefix + ('-dev-data' if kind == 'volume' else '-env'): issue})
                if name in expected or label in expected.values():
                    if expected.get(name) != label:
                        raise ValueError(f'{issue}: {kind} name/label conflict')
                    selected[kind].append(item)
        container_ids = {c['Id'] for c in selected['container']}
        volume_names = {v['Name'] for v in selected['volume']}
        for c in resources['container']:
            for mount in c.get('Mounts', []):
                if mount.get('Name') in volume_names and c['Id'] not in container_ids:
                    raise ValueError(f'{issue}: volume used by another container')
        for c in selected['container']:
            for mount in c.get('Mounts', []):
                if mount['Type'] != 'tmpfs' and not (
                    mount['Type'] == 'volume' and mount.get('Name') == prefix + '-dev-data'
                    and mount.get('Name') in volume_names
                    and mount.get('Destination') == '/var/lib/postgresql/data'
                ):
                    raise ValueError(f'{issue}: unexpected fixture mount')
            if set(c.get('NetworkSettings', {}).get('Networks', {})) - {prefix + '-env'}:
                raise ValueError(f'{issue}: unexpected fixture network')
        for network in selected['network']:
            if not network.get('Internal') or set(network.get('Containers', {})) - container_ids:
                raise ValueError(f'{issue}: network has unrelated consumers')
        plans.append({'issue': issue, 'eligible': True,
                      'container': [c['Id'] for c in selected['container']],
                      'container_names': [c['Name'].lstrip('/') for c in selected['container']],
                      'volume': [v['Name'] for v in selected['volume']],
                      'network': [n['Id'] for n in selected['network']]})
    return plans


def save(path, value):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def apply_one(base, item):
    issue = item['issue']
    journal = base / 'symphony/WORKFLOW.lifecycle.md.handoffs.json'
    # Re-read completion and Docker identities immediately before mutation.
    current = next(p for p in plan(json.loads(journal.read_text()), inventory()) if p['issue'] == issue)
    if current != item or not completed(json.loads(journal.read_text()), issue):
        raise ValueError('cleanup plan changed; retry from a fresh inventory')
    audit = base / 'symphony/fixture-cleanup' / (issue + '-' + str(time.time_ns()))
    audit.mkdir(parents=True, mode=0o700)
    save(audit / 'plan.json', item)
    unit = 'codexsymphony-' + issue.lower().replace('-', '') + '-db.service'
    unit_path = Path.home() / '.config/systemd/user' / unit
    expected = base / 'symphony' / (issue.lower().replace('-', '') + '-environment/broker.py')
    if unit_path.exists() or unit_path.is_symlink():
        if unit_path.is_symlink() or f'ExecStart=/usr/bin/python3 {expected}\n' not in unit_path.read_text():
            raise ValueError('unexpected fixture broker unit')
        (audit / 'broker.service').write_text(unit_path.read_text())
        run('systemctl', '--user', 'disable', '--now', unit)
        unit_path.unlink()
        run('systemctl', '--user', 'daemon-reload')
    for identifier in item['container']:
        logs = subprocess.run(['docker', 'logs', '--tail', '200', identifier],
                              capture_output=True, text=True, timeout=30, check=True)
        (audit / (identifier[:12] + '.log')).write_text((logs.stdout + logs.stderr)[-65536:])
        run('docker', 'stop', '--time', '10', identifier)
        run('docker', 'rm', identifier)
    for name in item['volume']:
        run('docker', 'volume', 'rm', name)
    for identifier in item['network']:
        run('docker', 'network', 'rm', identifier)
    save(audit / 'result.json', {'complete': True, 'issue': issue, 'time': time.time()})
    return str(audit)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    state = BASE / 'symphony'
    with (state / 'fixture-cleanup.lock').open('w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print(json.dumps({'deferred': 'another fixture collector holds the lock'}))
            return
        journal = json.loads((state / 'WORKFLOW.lifecycle.md.handoffs.json').read_text())
        for item in plan(journal, inventory()):
            if args.apply and item['eligible']:
                item['audit'] = apply_one(BASE, item)
            print(json.dumps(item), flush=True)


if __name__ == '__main__':
    main()

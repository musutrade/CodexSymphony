"""Fetch reviewed Git objects without creating a job checkout or touching HEAD."""
import hashlib
import json
from pathlib import Path
import re
import subprocess


def blob(root, revision, name):
    if not re.fullmatch('[0-9a-f]{40}', revision):
        raise ValueError('exact commit required')
    if Path(name).is_absolute() or '..' in Path(name).parts:
        raise ValueError('invalid protected path')
    row = subprocess.check_output(['git', '-C', root, 'ls-tree', '-z', revision, '--', name])
    metadata, separator, actual = row.partition(b'\t')
    fields = metadata.split()
    if not separator or actual != name.encode() + b'\0' or fields[:2] not in ([b'100644', b'blob'], [b'100755', b'blob']):
        raise ValueError('protected input must be a regular Git blob: ' + name)
    return subprocess.check_output(['git', '-C', root, 'cat-file', 'blob', fields[2].decode()])


def check_pins(root, revision, pins):
    for name, digest in pins.items():
        if hashlib.sha256(blob(root, revision, name)).hexdigest() != digest:
            raise ValueError('unapproved remote input: ' + name)


def select(root, revision, config):
    candidates = [{key: config[key] for key in ('protected_files', 'gate_approval')}]
    candidates += config.get('previous_deployments', [])
    for candidate in candidates:
        if set(candidate) != {'protected_files', 'gate_approval'}:
            raise ValueError('invalid reviewed deployment')
        approval = json.loads(Path(candidate['gate_approval']).read_text())
        if approval.get('execution_version') != 3 or approval['repository'] != str(root):
            continue
        try:
            for pins in (candidate['protected_files'], approval['trusted_files'], approval['config_files']):
                check_pins(root, revision, pins)
        except ValueError:
            continue
        return candidate, approval
    raise ValueError('source does not match a bounded reviewed deployment')


def prepare(run, config):
    approval = json.loads(Path(config['gate_approval']).read_text())
    root = Path(approval['repository'])
    volume = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
    if root != Path('/home/gem/CodexSymphony') or not root.is_mount() or not volume.is_mount():
        raise ValueError('fixed workspace mounts required')
    if root.stat().st_dev != volume.stat().st_dev:
        raise ValueError('workspace outside bounded storage')
    if config['repository'] != 'musutrade/CodexSymphony' or not re.fullmatch('[0-9a-f]{40}', run['head_sha']):
        raise ValueError('invalid remote source')
    subprocess.run(['git', '-C', root, '-c', 'core.hooksPath=/dev/null',
                    '-c', 'protocol.file.allow=never', '-c', 'protocol.ext.allow=never',
                    'fetch', '--quiet', '--no-tags', 'https://github.com/' + config['repository'] + '.git',
                    run['head_sha']], check=True)
    deployment, approval = select(root, run['head_sha'], config)
    config.update(deployment)
    return root, approval


def dependencies(root, revision, config):
    deps = Path(config['dependency_source'])
    for name in ('package.json', 'package-lock.json'):
        if blob(root, revision, 'web/angular/' + name) != (deps.parent / name).read_bytes():
            raise ValueError('frontend dependencies need host review')
    if deps != root / 'web/angular/node_modules' or not deps.is_dir() or deps.is_symlink():
        raise ValueError('fixed reviewed dependency directory required')



def state_home(config):
    approval = json.loads(Path(config['gate_approval']).read_text())
    volume = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
    if approval.get('execution_version') != 3 or not volume.is_mount():
        raise ValueError('bounded remote host approval and mount required')
    home = volume / 'evidence/remote-gate'
    if home.resolve() != home:
        raise ValueError('aliased remote state root')
    home.mkdir(mode=0o700, parents=True, exist_ok=True)
    return home

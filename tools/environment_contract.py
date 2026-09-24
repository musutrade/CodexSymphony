"""Shared environment contract for the host, development workspace and Gate.

Installed copies carry environment.lock.json beside this module. A workspace
cannot silently select a different contract from its installed host release.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

NAME = 'environment.lock.json'


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def load(repository=None):
    installed = Path(__file__).with_name(NAME)
    source = Path(repository) / NAME if repository else installed
    if not source.exists() and repository is None:
        source = Path(__file__).resolve().parents[1] / NAME
    receipt=Path(__file__).with_name('installed-files.json')
    if receipt.exists():
        for name,expected in json.loads(receipt.read_text()).items():
            if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
                raise ValueError('environment drift: installed file '+name)
    value = json.loads(source.read_text())
    if value.get('schema') != 'codexsymphony-environment/v1':
        raise ValueError('environment drift: unsupported contract schema')
    if repository and installed.exists() and value != json.loads(installed.read_text()):
        raise ValueError('environment drift: workspace contract differs from installed host release')
    return value


def projections(value):
    t = value['tools']
    return {'.node-version': t['node'] + '\n',
            'codex-version.lock': 'codex-cli ' + t['codex'] + '\n',
            'harness-gate-version.lock': 'harness-gate v' + t['gate'] + '\nrust-collector rust-collector-v' + t['rust_collector'] + '\n',
            'rust-toolchain.toml': '[toolchain]\nchannel = "' + t['rust'] + '"\nprofile = "minimal"\ncomponents = ["clippy", "rustfmt"]\n'}


def check_files(repository, value):
    for name, expected in projections(value).items():
        if (Path(repository) / name).read_text() != expected:
            raise ValueError('environment drift: generated file ' + name + '; run tools/environment_contract.py sync')
    for name,role in [('docker-compose.yml','test'),('docker-compose.dev.yml','dev')]:
        text=(Path(repository)/name).read_text()
        p=value['postgres'][role]
        for line in ['image: '+value['postgres']['image'], 'mem_limit: '+str(p['memory']),
                     'memswap_limit: '+str(p['memory_swap']), 'cpus: '+str(p['nano_cpus']/1_000_000_000)]:
            key=line.split(':',1)[0]+':'
            if [item.strip() for item in text.splitlines() if item.strip().startswith(key)] != [line]:
                raise ValueError('environment drift: '+name+' '+line)
    manager=json.loads((Path(repository)/'web/angular/package.json').read_text())['packageManager']
    if manager!='npm@'+value['tools']['npm']: raise ValueError('environment drift: frontend packageManager')
    # The Gate consumes the supplied database; its fallback must also be pinned.
    flow=tomllib.loads((Path(repository)/'.harness-gate/flow.toml').read_text())
    if flow['services']['test-postgres']['image'] != value['postgres']['image']:
        raise ValueError('environment drift: Gate database image')


def codex_bin(value):
    path = Path.home() / '.codex/packages/standalone/releases' / (value['tools']['codex'] + '-x86_64-unknown-linux-musl/bin')
    return Path('/opt/codex') if Path('/opt/codex/codex').is_file() else path


def tool_path(value):
    versions = Path.home() / '.local/share/harness-gate/versions'
    return ':'.join(map(str, [codex_bin(value), versions / ('v' + value['tools']['gate']) / 'bin',
                             versions / ('rust-collector-v' + value['tools']['rust_collector']) / 'bin',
                             Path.home() / '.cargo/bin'])) + ':/usr/local/bin:/usr/bin:/bin'


def test_environment(value):
    return dict(value['test_environment'])


def database_args(value, role='test'):
    p = value['postgres'][role]
    return ['--memory', str(p['memory']), '--memory-swap', str(p['memory_swap']),
            '--cpus', str(p['nano_cpus'] / 1_000_000_000)]


def check_database(state, value, role='test'):
    expected = value['postgres'][role]
    actual = {'memory': state['HostConfig']['Memory'], 'memory_swap': state['HostConfig']['MemorySwap'],
              'nano_cpus': state['HostConfig']['NanoCpus']}
    if state['Image'] != value['postgres']['image_id'] or actual != expected:
        raise ValueError('environment drift: PostgreSQL ' + role + ' expected ' + str(expected) + ', actual ' + str(actual) + ', image ' + state['Image'])
    return {'image': state['Image'], **actual}


def fingerprint(repository, environment=None):
    value = load(repository)
    check_files(repository, value)
    env = dict(os.environ if environment is None else environment)
    t = value['tools']
    probes = {'rustc': ('rustc', t['rust']), 'cargo': ('cargo', t['rust']), 'node': ('v', t['node']),
              'npm': ('', t['npm']), 'python3': ('Python', t['python']), 'codex': ('codex-cli', t['codex']),
              'cargo-llvm-cov': ('cargo-llvm-cov', t['cargo_llvm_cov']),
              'rustfmt': ('rustfmt', t['rustfmt']), 'clippy-driver': ('clippy', t['clippy']),
              'harness-gate': ('harness-gate', t['gate']),
              'harness-gate-rust-collector': ('harness-gate-rust-collector', t['rust_collector'])}
    actual = {}
    for name, (prefix, version) in probes.items():
        binary = shutil.which(name, path=env.get('PATH'))
        if not binary:
            raise ValueError('environment drift: missing ' + name)
        arguments=[binary,'llvm-cov','--version'] if name=='cargo-llvm-cov' else [binary,'--version']
        result = subprocess.check_output(arguments, cwd=repository, env=env, text=True, timeout=20).strip()
        expected = (prefix + (' ' if prefix and prefix != 'v' else '') + version)
        if result != expected and not (name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver') and result.startswith(expected + ' (')):
            raise ValueError(f'environment drift: {name}: expected {expected}, actual {result}')
        # rustup proxy bytes alone do not identify the selected compiler.
        if name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
            binary = subprocess.check_output(['rustup', 'which', name], cwd=repository, env=env, text=True, timeout=20).strip()
        actual[name] = {'version': result, 'sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
        if actual[name]['sha256'] != value['tool_sha256'][name]:
            raise ValueError('environment drift: '+name+' binary SHA-256 differs from manifest')
    for name, expected in value['test_environment'].items():
        if env.get(name) != expected:
            raise ValueError(f'environment drift: {name}: expected {expected}, actual {env.get(name)}')
    identity = {'contract': digest(value), 'tools': actual, 'postgres': value['postgres'], 'test_environment': value['test_environment']}
    return {'schema': 'codexsymphony-environment-fingerprint/v1', 'fingerprint': digest(identity), **identity}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['check', 'sync', 'shell'])
    parser.add_argument('--repository', type=Path, default=Path.cwd())
    args = parser.parse_args()
    value = load(args.repository)
    if args.operation == 'shell':
        import shlex
        for name,value in dict(test_environment(value), PATH=tool_path(value)).items():
            print('export '+name+'='+shlex.quote(value))
    elif args.operation == 'sync':
        for name, text in projections(value).items():
            (args.repository / name).write_text(text)
        import re
        for name,role in [('docker-compose.yml','test'),('docker-compose.dev.yml','dev')]:
            path=args.repository/name
            text=path.read_text()
            resources=value['postgres'][role]
            for key,setting in {'image':value['postgres']['image'],'mem_limit':resources['memory'],
                                'memswap_limit':resources['memory_swap'],'cpus':resources['nano_cpus']/1_000_000_000}.items():
                text,count=re.subn(r'(?m)^    '+key+r':.*$', '    '+key+': '+str(setting),text)
                if count!=1: raise ValueError('expected one compose projection: '+key)
            path.write_text(text)
        path=args.repository/'.harness-gate/flow.toml'
        text,count=re.subn(r'(?m)^image = .*$', 'image = '+json.dumps(value['postgres']['image']),path.read_text())
        if count!=1: raise ValueError('expected one Gate image projection')
        path.write_text(text)
    else:
        print(json.dumps(fingerprint(args.repository), indent=2))


if __name__ == '__main__':
    main()

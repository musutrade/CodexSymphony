"""Linux host isolation: expose only approved code, tools and per-run outputs."""
import os
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract
import subprocess
import uuid

HOME = Path('/home/gem')

def command(argv, *, run, repository, plugins, writable=(), readonly=(), mounts=(), environment=None, cwd=None):
    run = Path(run).resolve()
    policy = contract.load(repository)
    codex = contract.codex_bin(policy)
    if not codex.is_dir():
        codex = Path('/opt/codex')
    if not (codex/'codex').is_file():
        raise ValueError('pinned Codex runtime mount missing')
    cargo = run / 'cargo-home'
    cargo.mkdir(exist_ok=True)
    temporary = run / 'tmp'
    temporary.mkdir(exist_ok=True)
    cache = run / 'target/sccache'
    cache.mkdir(parents=True, exist_ok=True)
    # Dedicated AppArmor transition permits nested user namespaces without
    # weakening the inner command's filesystem, PID or network isolation.
    args = ['/usr/local/libexec/codexsymphony/bwrap', '--die-with-parent', '--new-session', '--unshare-user', '--unshare-pid',
            '--ro-bind', '/usr', '/usr', '--ro-bind', '/etc', '/etc', '--tmpfs', '/etc/codex',
            '--ro-bind', str(codex), '/opt/codex',
            '--symlink', 'usr/bin', '/bin', '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
            '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/run', '--bind', str(temporary), '/tmp',
            '--dir', str(HOME), '--bind', str(cargo), str(HOME / '.cargo'),
            '--ro-bind', str(HOME / '.cargo/registry'), str(HOME / '.cargo/registry'),
            '--ro-bind', str(HOME / '.cargo/bin'), str(HOME / '.cargo/bin'),
            '--ro-bind', str(HOME / '.rustup'), str(HOME / '.rustup'),
            '--ro-bind', str(repository), str(repository),
            '--ro-bind', str(plugins), str(plugins)]
    for path in readonly:
        args += ['--ro-bind', str(path), str(path)]
    for path in writable:
        args += ['--bind', str(path), str(path)]
    for source, target in mounts:
        args += ['--ro-bind', str(source), str(target)]
    # A private copy of a reviewed seed; project code cannot write the seed.
    args += ['--bind', str(cache), policy['test_environment']['SCCACHE_DIR']]
    env = {'HOME': str(HOME), 'CARGO_HOME': str(HOME / '.cargo'), 'RUSTUP_HOME': str(HOME / '.rustup'),
           'PATH': '/opt/codex:' + str(HOME / '.cargo/bin') + ':/usr/local/bin:/usr/bin:/bin',
           'LANG': 'C.UTF-8', 'TZ': 'UTC', 'CARGO_NET_OFFLINE': 'true',
           'CARGO_TARGET_DIR': str(run / 'target')}
    env.update(environment or {})
    env.update(contract.test_environment(policy))
    args += ['--clearenv']
    for name, value in env.items(): args += ['--setenv', name, value]
    args += ['--chdir', str(cwd or repository), '--', *map(str, argv)]
    # Read stats before the PID namespace tears down its private cache daemon.
    # Compilation stays in this namespace, never in a host credential process.
    index = args.index('--', args.index('--chdir'))
    execution = args[index+1:]
    stats = '/tmp/sccache-' + uuid.uuid4().hex + '.json'
    wrapper = ('import os,subprocess,sys\nfrom pathlib import Path\n'
               'try:\n code=subprocess.call(sys.argv[1:])\n'
               'finally:\n'
               ' if Path(os.environ["SCCACHE_SERVER_UDS"]).exists():\n'
               '  result=subprocess.run([os.environ["RUSTC_WRAPPER"],"--show-stats","--stats-format","json"],capture_output=True,timeout=10)\n'
               f'  if result.returncode==0: Path({stats!r}).write_bytes(result.stdout)\n'
               '  subprocess.run([os.environ["RUSTC_WRAPPER"],"--stop-server"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)\n'
               'sys.exit(code)\n')
    return args[:index+1] + ['python3', '-c', wrapper, *execution]

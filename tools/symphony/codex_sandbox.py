#!/usr/bin/env python3
"""Launch the pinned Codex with no mount of GitHub keys or host gate state."""
import os
from pathlib import Path
import sys

HOME=Path('/home/gem')
BASE=HOME/'.local/share/codexsymphony'
WORKSPACES=BASE/'workspaces'
CODEX=HOME/'.codex/packages/standalone/releases/0.154.0-x86_64-unknown-linux-musl/bin'


def resolver_mount(resolver=Path('/etc/resolv.conf')):
    # /etc/resolv.conf commonly points into /run, which we deliberately hide.
    # Preserve only the resolved DNS configuration, not the host runtime tree.
    target=resolver.resolve(strict=True)
    if not target.is_file():
        raise ValueError('host DNS resolver configuration is not a regular file')
    return ['--ro-bind',str(target),str(target)]


def command(argv, state_home=None):
    cwd=Path.cwd().resolve()
    if not cwd.is_relative_to(WORKSPACES) or cwd==WORKSPACES:
        raise ValueError('Codex must run in an assigned project workspace')
    cargo=cwd/'.agent-cargo';cargo.mkdir(exist_ok=True)
    temporary=cwd/'.agent-tmp';temporary.mkdir(exist_ok=True)
    auth=BASE/'codex-home'
    args=['/usr/local/libexec/codexsymphony/bwrap','--die-with-parent','--new-session','--unshare-user','--unshare-pid',
          '--ro-bind','/usr','/usr','--ro-bind','/etc','/etc',
          '--symlink','usr/bin','/bin','--symlink','usr/lib','/lib','--symlink','usr/lib64','/lib64',
          '--proc','/proc','--dev','/dev','--tmpfs','/run',*resolver_mount(),
          '--bind',str(temporary),'/tmp','--dir',str(HOME),
          '--bind',str(cwd),str(cwd),'--ro-bind',str(cwd/'.git'),str(cwd/'.git'),
          '--bind',str(state_home or auth),str(auth),'--ro-bind',str(CODEX),'/opt/codex',
          '--bind',str(cargo),str(HOME/'.cargo'),
          '--ro-bind',str(HOME/'.cargo/bin'),str(HOME/'.cargo/bin'),
          '--ro-bind',str(HOME/'.cargo/registry'),str(HOME/'.cargo/registry'),
          '--ro-bind',str(HOME/'.rustup'),str(HOME/'.rustup'),
          '--ro-bind',str(HOME/'.agents/skills'),str(HOME/'.agents/skills'),
          '--ro-bind',str(HOME/'.codex/skills'),str(HOME/'.codex/skills'),
          '--ro-bind',str(HOME/'.local/share/harness-gate'),str(HOME/'.local/share/harness-gate')]
    provision=BASE/'symphony'/(cwd.name.lower().replace('-', '')+'-environment')
    if (provision/'requirements.toml').is_file():
        args+=['--ro-bind',str(provision/'requirements.toml'),'/etc/codex/requirements.toml',
               '--ro-bind',str(provision/'arc-admin'),str(HOME/'arc-admin'),
               '--ro-bind',str(provision/'client'),'/opt/'+cwd.name.lower().replace('-', '')+'-env',
               '--ro-bind',str(HOME/'.cache/ms-playwright'),str(HOME/'.cache/ms-playwright')]
        args+=['--symlink','/opt/'+cwd.name.lower().replace('-', '')+'-env','/opt/symphony-env']
    env={'HOME':str(HOME),'CODEX_HOME':str(auth),'CARGO_HOME':str(HOME/'.cargo'),
         'RUSTUP_HOME':str(HOME/'.rustup'),'CARGO_TARGET_DIR':str(cwd/'target'),
         'PATH':'/opt/codex:/home/gem/.local/share/harness-gate/versions/v0.4.5/bin:/home/gem/.local/share/harness-gate/versions/rust-collector-v0.1.0-rc.6/bin:/home/gem/.cargo/bin:/usr/local/bin:/usr/bin:/bin',
         'LANG':'C.UTF-8','TZ':'UTC','NO_COLOR':'1','HTTP_PROXY':'http://127.0.0.1:7890',
         'HTTPS_PROXY':'http://127.0.0.1:7890','NO_PROXY':'127.0.0.1,localhost,::1'}
    if (provision/'requirements.toml').is_file():
        env['npm_config_cache']=str(cwd/'.agent-env/npm-cache')
    args+=['--clearenv']
    for name,value in env.items():args+=['--setenv',name,value]
    return args+['--chdir',str(cwd),'--',*argv]


def reviewed_runtime_command(state):
    """Fixed reviewed test binary, isolated from host networking and credentials."""
    import hashlib
    import json
    cwd = Path.cwd().resolve()
    provision = BASE/'symphony'/(cwd.name.lower().replace('-', '')+'-environment')
    reviewed = provision/'reviewed-runtime'
    manifest = json.loads((reviewed/'manifest.json').read_text())
    if manifest['workspace'] != str(cwd):
        raise ValueError('reviewed Runtime workspace mismatch')
    for name, expected in manifest['sources'].items():
        path = cwd/name
        if path.resolve() != path.absolute() or not path.is_relative_to(cwd):
            raise ValueError('reviewed Runtime source path mismatch')
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('Runtime source changed; rebuild and review installation: '+name)
    for name in ('test', 'supervisor'):
        path = reviewed/name
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != manifest['binaries'][name]:
            raise ValueError('reviewed Runtime binary mismatch')
    args = command(['/opt/reviewed-runtime/test', '--ignored', '--exact',
                    'real_runtime_transport_and_supervision', '--nocapture'], state_home=state)
    at = args.index('--chdir')
    args[at:at] = ['--unshare-net', '--ro-bind', str(reviewed), '/opt/reviewed-runtime',
                  '--setenv', 'SYMPHONY_REVIEWED_RUNTIME_TEST', '1',
                  '--setenv', 'SYMPHONY_REVIEWED_SUPERVISOR', '/opt/reviewed-runtime/supervisor',
                  '--setenv', 'NO_PROXY', '127.0.0.1,localhost,::1']
    return args

if __name__=='__main__':
    if sys.argv[1:] in (['--runtime-readiness-app-server'], ['--runtime-product-acceptance']):
        # Host-only fixed probe entry. Mount empty disposable state instead of
        # shared authentication; preserve the same launcher and requirements.
        import tempfile
        import subprocess
        import signal
        def stop_probe(_signal, _frame):
            raise SystemExit(0)
        signal.signal(signal.SIGTERM, stop_probe)
        with tempfile.TemporaryDirectory(prefix='codexsymphony-runtime-') as state:
            args = (reviewed_runtime_command(state) if sys.argv[1] == '--runtime-product-acceptance'
                    else command(['/opt/codex/codex', 'app-server'], state_home=state))
            result = subprocess.run(args)
        sys.exit(result.returncode)
    args=command(['/opt/codex/codex',*sys.argv[1:]])
    os.execv(args[0],args)

#!/usr/bin/env python3
"""Launch the pinned Codex with no mount of GitHub keys or host gate state."""
import os
from pathlib import Path
import sys

HOME=Path('/home/gem')
BASE=HOME/'.local/share/codexsymphony'
WORKSPACES=BASE/'workspaces'
CODEX=HOME/'.codex/packages/standalone/releases/0.154.0-x86_64-unknown-linux-musl/bin'


def command(argv):
    cwd=Path.cwd().resolve()
    if not cwd.is_relative_to(WORKSPACES) or cwd==WORKSPACES:
        raise ValueError('Codex must run in an assigned project workspace')
    cargo=cwd/'.agent-cargo';cargo.mkdir(exist_ok=True)
    temporary=cwd/'.agent-tmp';temporary.mkdir(exist_ok=True)
    auth=BASE/'codex-home'
    args=['/usr/local/libexec/codexsymphony/bwrap','--die-with-parent','--new-session','--unshare-user','--unshare-pid',
          '--ro-bind','/usr','/usr','--ro-bind','/etc','/etc',
          '--symlink','usr/bin','/bin','--symlink','usr/lib','/lib','--symlink','usr/lib64','/lib64',
          '--proc','/proc','--dev','/dev','--tmpfs','/run',
          '--bind',str(temporary),'/tmp','--dir',str(HOME),
          '--bind',str(cwd),str(cwd),'--ro-bind',str(cwd/'.git'),str(cwd/'.git'),
          '--bind',str(auth),str(auth),'--ro-bind',str(CODEX),'/opt/codex',
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

if __name__=='__main__':
    args=command(['/opt/codex/codex',*sys.argv[1:]])
    os.execv(args[0],args)

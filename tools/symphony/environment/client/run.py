"""Supply project fixture settings; execute the ordinary command directly."""
import os
from pathlib import Path
import subprocess
import sys


def main():
    env = os.environ.copy()
    env['PATH']='/opt/gh12-env/bin:'+env.get('PATH','/usr/bin:/bin')
    env['NO_PROXY']=env['no_proxy']='127.0.0.1,localhost,::1,172.30.212.2,172.30.212.3'
    for role,host in [('test','172.30.212.2'),('dev','172.30.212.3')]:
        user='codexsymphony_'+role
        env['TEST_DATABASE_URL' if role=='test' else 'DEV_DATABASE_URL']=f'postgres://{user}:{user}@{host}:5432/{user}?sslmode=disable'
    env['DATABASE_URL']=env['TEST_DATABASE_URL']
    command=sys.argv[1:]
    if not command:raise SystemExit('usage: run.py COMMAND [ARGS]')
    if command[0]=='psql':
        pg=Path(__file__).parent/'pg'
        command=[str(pg/'ld-musl-x86_64.so.1'),'--library-path',str(pg),str(pg/'psql'),*command[1:]]
    return subprocess.call(command,env=env)


if __name__=='__main__':sys.exit(main())

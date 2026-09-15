import json, os, select, subprocess, sys, time
from pathlib import Path

BASE = Path(__file__).parent
ROOT = Path('/home/gem/.local/share/codexsymphony/workspaces/GH-12')

def execute(command, timeout=120):
    with (BASE/f'preflight-{os.getpid()}.stderr').open('w') as err:
        p = subprocess.Popen([str(BASE.parent/'codex-sandbox'), 'app-server'], cwd=ROOT,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err)
        pending = b''
        def call(identity, method, params):
            nonlocal pending
            p.stdin.write((json.dumps({'id':identity,'method':method,'params':params})+'\n').encode())
            p.stdin.flush()
            deadline=time.monotonic()+timeout+20
            while time.monotonic()<deadline:
                if b'\n' not in pending:
                    if not select.select([p.stdout],[],[],1)[0]: continue
                    block=os.read(p.stdout.fileno(),65536)
                    if not block: raise RuntimeError('app-server exited; see preflight.stderr')
                    pending+=block
                while b'\n' in pending:
                    line,pending=pending.split(b'\n',1)
                    reply=json.loads(line)
                    if reply.get('id')==identity:
                        if 'error' in reply: raise RuntimeError(reply['error'])
                        return reply['result']
            raise TimeoutError(method)
        try:
            call(1,'initialize',{'clientInfo':{'name':'gh12-environment-preflight','version':'1'},'capabilities':{'experimentalApi':True}})
            result=call(2,'command/exec',{'command':command,'cwd':str(ROOT),'sandboxPolicy':{'type':'workspaceWrite','writableRoots':[],'networkAccess':True},'timeoutMs':timeout*1000})
            return result
        finally:
            p.terminate()
            try: p.wait(timeout=5)
            except subprocess.TimeoutExpired: p.kill();p.wait()

if __name__=='__main__':
    result=execute(sys.argv[1:])
    print(json.dumps(result,ensure_ascii=False))
    sys.exit(result['exitCode'])

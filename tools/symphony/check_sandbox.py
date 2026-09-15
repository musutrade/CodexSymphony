#!/usr/bin/env python3
"""Validate the installed nested sandbox through app-server command/exec, without a model."""
import json,select,subprocess,time
from pathlib import Path
root=Path('/home/gem/.local/share/codexsymphony/workspaces/isolation-preflight')
base=Path('/home/gem/.local/share/codexsymphony/symphony')
root.mkdir(parents=True,exist_ok=True)
subprocess.run(['git','init','--quiet',root],check=True)
script="""import errno,json
from pathlib import Path
import subprocess
core_version=subprocess.check_output(["harness-gate","--version"],text=True).strip()
assert core_version=="harness-gate 0.4.5"
p=Path('.git/environment-acceptance-canary')
try:
 with p.open('x') as probe:probe.write('probe')
 p.unlink();readonly=False
except OSError as error:
 assert error.errno==errno.EROFS,error
 readonly=True
r={'git_readonly':readonly,'app_key_visible':Path('/home/gem/.secrets/my-disposable-bot.2026-09-08.private-key.pem').exists(),'host_approval_visible':Path('/home/gem/.local/share/codexsymphony/gate-host/approval.json').exists()}
assert r=={'git_readonly':True,'app_key_visible':False,'host_approval_visible':False}
r['core_version']=core_version
print(json.dumps(r))
"""
with (base/'command-preflight.stderr').open('w') as err:
 p=subprocess.Popen([str(base/'codex-sandbox'),'app-server'],cwd=root,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
 try:
  def call(identity,method,params):
   p.stdin.write(json.dumps({'id':identity,'method':method,'params':params})+'\n');p.stdin.flush()
   deadline=time.monotonic()+30
   while time.monotonic()<deadline:
    if select.select([p.stdout],[],[],1)[0]:
     line=p.stdout.readline()
     if not line:break
     reply=json.loads(line)
     if reply.get('id')==identity:
      assert 'error' not in reply,reply.get('error')
      return reply['result']
   raise RuntimeError('RPC timeout: '+method)
  call(1,'initialize',{'clientInfo':{'name':'codexsymphony-preflight','version':'1'},'capabilities':{'experimentalApi':True}})
  result=call(2,'command/exec',{'command':['python3','-c',script],'cwd':str(root),'sandboxPolicy':{'type':'workspaceWrite','writableRoots':[],'networkAccess':True},'timeoutMs':20000})
  assert result['exitCode']==0,result
  proof=json.loads(result['stdout']);print(json.dumps(proof))
  (base/'command-preflight.json').write_text(json.dumps({'command_exec':'PASS','codex':'0.154.0','proof':proof},indent=2)+'\n')
 finally:
  p.terminate()
  try:p.wait(timeout=5)
  except subprocess.TimeoutExpired:p.kill();p.wait()

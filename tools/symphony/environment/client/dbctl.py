"""Request a fixed GH-12 test fixture operation from the host."""
import json, os, sys, time, uuid
from pathlib import Path

def request(action,role):
    if action not in ['status','recreate'] or role not in ['test','dev']:
        raise ValueError('usage: dbctl.py status|recreate test|dev')
    base=Path('/home/gem/.local/share/codexsymphony/workspaces/GH-12/.agent-env/requests')
    identity=uuid.uuid4().hex
    temporary=base/(identity+'.tmp')
    temporary.write_text(json.dumps({'action':action,'role':role}))
    temporary.rename(base/(identity+'.request.json'))
    result=base/(identity+'.result.json')
    deadline=time.monotonic()+120
    while time.monotonic()<deadline:
        if result.exists():
            try: value=json.loads(result.read_text())
            except json.JSONDecodeError: time.sleep(.1);continue
            if not value['ok']: raise RuntimeError(value['error'])
            return value
        time.sleep(.2)
    raise TimeoutError('Host fixture operation timed out; inspect result before retrying')

if __name__=='__main__': print(json.dumps(request(*sys.argv[1:])))

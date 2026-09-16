"""Request the host's fixed isolated Runtime command probe; not product acceptance."""
import json,time,uuid
from pathlib import Path


def probe():
    spool=Path('/home/gem/.local/share/codexsymphony/workspaces/GH-12/.agent-env/requests')
    requested_at=time.time()
    identity=uuid.uuid4().hex
    temporary=spool/(identity+'.tmp')
    temporary.write_text(json.dumps({'action':'runtime-command-readiness'}))
    temporary.rename(spool/(identity+'.request.json'))
    result=spool/(identity+'.result.json')
    deadline=time.monotonic()+90
    while time.monotonic()<deadline:
        if result.exists():
            try:value=json.loads(result.read_text())
            except json.JSONDecodeError:time.sleep(.1);continue
            if not value.get('ok'):raise RuntimeError(value.get('error','probe failed'))
            # Canonical evidence is host-owned and mounted read-only.
            canonical=json.loads((Path(__file__).parent/'runtime-command-readiness.json').read_text())
            if canonical['checked_at']<requested_at:raise RuntimeError('stale probe receipt')
            if canonical['sample_id']!=value['sample_id']:raise RuntimeError('probe receipt changed; request a fresh sample')
            return canonical
        time.sleep(.2)
    raise TimeoutError('Host execution readiness did not respond within 90 seconds')

if __name__=='__main__':print(json.dumps(probe(),indent=2))

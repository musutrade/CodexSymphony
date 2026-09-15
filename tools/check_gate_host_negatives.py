#!/usr/bin/env python3
"""Reject tampered/incomplete full-run inputs while retaining accepted evidence."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
sys.path.insert(0,str(Path(__file__).parent/'quality-host'))
from replay import broker


def write(path,value): path.write_text(json.dumps(value,indent=2)+'\n')
def load(path): return json.loads(path.read_text())

def check(run):
    root=run/'workspace';runtime=root/'.harness-gate/runtime'
    state=runtime/'ci-state.json';keys=runtime/'trusted-keys.json'
    report=load(root/'.harness-gate/reports/test_result.json')
    assert report['passed'] and report['evidence_complete']
    output=run/'negative-checks';output.mkdir()
    command=['harness-gate','quality','collect','--repository-root',str(root),'--state',str(state),'--trusted-keys',str(keys),'--output',str(output/'collection.json')]
    results=[]
    def reject(name,command,reason=None):
        value=subprocess.run(command,capture_output=True,text=True)
        (output/(name+'.stdout')).write_text(value.stdout);(output/(name+'.stderr')).write_text(value.stderr)
        assert value.returncode!=0,'accepted '+name
        if reason: assert reason in value.stderr,value.stderr
        results.append({'case':name,'rejected':True,'exit':value.returncode})
    evidence=root/'.harness-gate/reports/evidence';retained=run/'accepted-evidence-during-negatives'
    original_state=state.read_bytes();request_path=runtime/'backend-request.json';original_request=request_path.read_bytes()
    evidence.rename(retained);evidence.mkdir()
    # Keep prior diagnostic Core state, but ensure this probe reaches the host
    # ledger rather than being rejected by a preceding diagnostic collect.
    core_ledger=root/'.harness-gate/collector-nonces'
    prior_ledger=output/'prior-core-ledger'
    if core_ledger.exists(): core_ledger.rename(prior_ledger)
    guard=broker(run,Path('/home/gem/.local/share/codexsymphony/gate-host/nonces'));guard.__enter__()
    try:
        audit=run/'nonce-events.jsonl'
        before=len(audit.read_text().splitlines())
        reject('replay',command)
        events=[json.loads(line) for line in (run/'nonce-events.jsonl').read_text().splitlines()]
        assert len(events)>before,'replay did not contact the host guard'
        assert events[-1].get('reason')=='nonce has already been used','replay did not reach durable host guard'
        request=load(request_path);request['nonce']+='-changed';write(request_path,request)
        changed=load(state);changed['config_files']['.harness-gate/runtime/backend-request.json']=hashlib.sha256(request_path.read_bytes()).hexdigest();write(state,changed)
        reject('signature-tampering',command,'signature')
        request_path.write_bytes(original_request);state.write_bytes(original_state)
        changed=load(state);changed['expected']['commit']='f'*40;write(state,changed)
        reject('stale-context',command,'stale collector config identity')
    finally:
        guard.__exit__(None,None,None)
        request_path.write_bytes(original_request);state.write_bytes(original_state)
        if core_ledger.exists(): core_ledger.rename(output/'generated-core-ledger')
        if prior_ledger.exists(): prior_ledger.rename(core_ledger)
        assert not list(evidence.iterdir()),'a rejected request published artifacts'
        evidence.rmdir();retained.rename(evidence)
    compiled=load(run/'signed/compiled.json')
    for key in ('project','policy','expected'): write(output/(key+'.json'),compiled[key])
    write(output/'evidence.json',report['quality']['evidence'])
    evaluate=['harness-gate','quality','evaluate','--source-root',str(root),'--artifact-root',str(evidence),'--output',str(output/'report.json')]
    for key in ('project','policy','expected','evidence'):evaluate+=['--'+key,str(output/(key+'.json'))]
    artifact=evidence/report['quality']['evidence'][0]['artifacts'][0]['path'];original=artifact.read_bytes()
    try:
        artifact.write_bytes(original+b' ');reject('artifact-tampering',evaluate)
    finally: artifact.write_bytes(original)
    incomplete=[r for r in report['quality']['evidence'] if r['collector']['name']!='http-json-contract']
    write(output/'evidence.json',incomplete);reject('missing-contract-producer',evaluate)
    write(output/'evidence.json',report['quality']['evidence'])
    result={'schema':'codexsymphony-full-gate-negative-checks/v1','cases':results}
    write(output/'acceptance.json',result);print(json.dumps(result))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('run',type=Path);check(parser.parse_args().run)

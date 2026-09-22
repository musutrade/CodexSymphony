"""One-time bootstrap of reviewed project bindings; verification never calls this."""
import json
from pathlib import Path
import subprocess
from capture import TS, node, write

def configure(root, requests, identities):
    frontend=requests['frontend'];files=[s for s in frontend['parameters']['subjects'] if s['kind']=='file/v1']
    script="const fs=require('fs'),p=require(process.argv[1]);const q=JSON.parse(fs.readFileSync(0,'utf8'));console.log(JSON.stringify(q.map(s=>p.inventory(s.path,fs.readFileSync(s.absolute,'utf8')).declarationOnly)));"
    flags=json.loads(subprocess.check_output(['node','-e',script,str(TS/'measure.cjs')],input=json.dumps([{'path':s['path'],'absolute':str(root/s['path'])} for s in files]),text=True))
    declarations={s['id'] for s,flag in zip(files,flags,strict=True) if flag}
    groups={'backend-functions':requests['backend']['parameters']['subjects'],
            'frontend-files':[s for s in files if s['id'] not in declarations],
            'frontend-declarations':[s for s in files if s['id'] in declarations],
            'frontend-functions':[s for s in frontend['parameters']['subjects'] if s['kind']=='function/v1'],
            'api-provider':[s for s in requests['frontend-api']['parameters']['subjects'] if s['component']=='backend'],
            'api-consumer':[s for s in requests['frontend-api']['parameters']['subjects'] if s['component']=='frontend']}
    assert all(groups.values()),'bootstrap expects the complete initial production inventory'
    selected={'backend':['backend-functions'],'frontend':['frontend-files','frontend-declarations','frontend-functions'],'frontend-api':['api-provider','api-consumer']}
    quality='''version = 1
[limits]
max_artifact_bytes = 1073741824
[project]
id = "codexsymphony"
name = "codexsymphony"
[components.backend]
flow_components = ["backend"]
source_roots = ["apps/server/src", "api"]
artifact_root = ".harness-gate/reports/evidence"
[components.frontend]
flow_components = ["frontend"]
source_roots = ["web/angular/src"]
artifact_root = ".harness-gate/reports/evidence"
'''
    for name,subjects in groups.items():
        collector = 'backend' if name=='backend-functions' else 'frontend-api' if name.startswith('api-') else 'frontend'
        quality+=f'''[subjects.{name}]
component = "{subjects[0]['component']}"
kind = "{name}"
selection = {{kind = "discovery", collector = "{collector}", query = "{name}/v1"}}
'''
    quality+='''[relationships.frontend-api]
kind = "api-contract"
from = {kind = "subject", id = "api-provider"}
to = {kind = "subject", id = "api-consumer"}
'''
    for collector,groups_ in selected.items():
        quality+=f'''[collectors.{collector}]
protocol = "harness-collector-request/v1"
request = ".harness-gate/runtime/{collector}-request.json"
'''
        for name in (['frontend-api'] if collector=='frontend-api' else groups_):
            kind='relationship' if collector=='frontend-api' else 'subject'
            for metric in requests[collector]['requested_capabilities']:
                quality+=f'''[[collectors.{collector}.produces]]
target = {{kind = "{kind}", id = "{name}"}}
capability = "{metric}"
series = "{identities[collector]['id']}"
'''
    rules={k:[] for k in selected}
    assignments=[('backend','backend-functions','coverage.line','backend.coverage.line'),('backend','backend-functions','coverage.region','backend.coverage.region'),('backend','backend-functions','risk.crap','backend.risk.crap'),
                 ('frontend','frontend-files','coverage.line','frontend.coverage.line'),('frontend','frontend-functions','coverage.line','frontend.functions.coverage.line'),('frontend','frontend-functions','coverage.function','frontend.coverage.function'),('frontend','frontend-functions','risk.crap','frontend.risk.crap')]
    assignments += [('frontend-api','frontend-api',m,'frontend-api.'+m) for m in requests['frontend-api']['requested_capabilities']]
    for collector,group,metric,name in assignments:
        if metric=='risk.crap': operator,limit='le',{'type':'rational','numerator':10,'denominator':1}
        elif metric.startswith('coverage.'): operator,limit='ge',{'type':'ratio','covered':80,'total':100}
        elif metric=='contract.breaking_changes': operator,limit='le',{'type':'count','value':0}
        else: operator,limit='eq',{'type':'boolean','value':metric=='contract.compatible'}
        kind='relationship' if collector=='frontend-api' else 'subject'
        rule={'id':name,'scope':{'kind':kind,kind:group},'metric':metric,'operator':operator,'limit':limit,'required':True,'on_violation':'fail','remediation_classes':['review_retained_evidence']}
        rules[collector].append(rule)
        quality+=f'''[policies."{name}"]
policy_file = ".harness-gate/packs/{collector}/policy.json"
rule = "{name}"
expectation = {{target = {{kind = "{kind}", id = "{group}"}}, capability = "{metric}", series = "{identities[collector]['id']}"}}
'''
    names=[r['id'] for rules_ in rules.values() for r in rules_]
    for profile in ('ci','full'):
        quality+=f'''[profiles.{profile}]
assurance = "complete"
collectors = ["backend", "frontend", "frontend-api"]
policies = {json.dumps(names)}
[profiles.{profile}.workflow]
state = ".harness-gate/runtime/{profile}-state.json"
trusted_keys = ".harness-gate/runtime/trusted-keys.json"
'''
    quality+='''[profiles.hook]
assurance = "partial"
collectors = []
policies = []
[profiles.hook.workflow]
state = ".harness-gate/runtime/hook-state.json"
trusted_keys = ".harness-gate/runtime/trusted-keys.json"
[baseline]
required = false
provider = {kind = "none"}
[reporting]
output = ".harness-gate/reports/quality"
formats = ["human", "json"]
'''
    (root/'.harness-gate/quality.toml').write_text(quality)
    for collector in selected:
        write(root/f'.harness-gate/packs/{collector}/policy.json',{'schema':'harness-policy/v1','rules':rules[collector]})
        write(root/f'.harness-gate/packs/{collector}/capabilities.json',{'series':identities[collector],'states':{m:'supported' for m in requests[collector]['requested_capabilities']}})
    return groups

def state(requests,identities,groups,profile):
    context=requests['backend']['context'];target=context['target']
    components={}
    for name,boundaries in [('backend',[('production','apps/server/src'),('contract','api')]),('frontend',[('production','web/angular/src')])]:
        components[name]={'id':name,'path':'.' if name=='backend' else 'web/angular','metadata':{},'targets':[{'id':target,'boundaries':[b for b,p in boundaries],'metadata':{}}],
                          'source_boundaries':[{'id':b,'path':p,'role':'production','metadata':{}} for b,p in boundaries]}
    return {'schema':'quality-trusted-state/v1','profile':profile,'expected':context,'components':components,'subjects':groups,
            'subject_kinds':{name:subjects[0]['kind'] for name,subjects in groups.items()},'relationship_kinds':{'api-contract':'generated_from/v1'},
            'series':identities,'artifact_root':'.harness-gate/reports/evidence','artifacts':{},'selection':{'changed_subject':sorted(groups),'critical_subject':[]},'mappings':None,'exceptions':[]}

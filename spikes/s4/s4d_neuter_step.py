"""S4d: keep the step defined but neuter its command (program -> true). Schema-valid bypass?"""
import json, pathlib, re, subprocess, sys, tempfile
HG=sys.argv[1]
def sh(*a,cwd=None,check=True):
    r=subprocess.run(list(a),cwd=cwd,text=True,capture_output=True)
    if check and r.returncode!=0: raise RuntimeError(f"{a[:2]} rc={r.returncode}: {r.stderr}")
    return r
def gate(repo,*args):
    r=subprocess.run([HG,"--project-root",str(repo),*args],text=True,capture_output=True)
    t=r.stdout+r.stderr
    return {"rc":r.returncode,
            "summary":next((l.strip() for l in t.splitlines() if l.startswith("TEST_SUMMARY")),None),
            "err":next((l.strip()[:150] for l in t.splitlines() if l.strip().startswith(("ERROR","error"))),None),
            "ran":[l.strip() for l in t.splitlines() if l.strip().startswith("[")]}
work=pathlib.Path(tempfile.mkdtemp(prefix="s4d-")); repo=work/"repo"; repo.mkdir(); F={}
sh("git","init","-q","-b","main",cwd=repo); sh("git","config","user.email","a@b.c",cwd=repo); sh("git","config","user.name","a",cwd=repo)
sh(HG,"init","--preset","generic",cwd=repo)
(repo/"app.md").write_text("# app\n"); sh("git","add",".",cwd=repo); sh("git","commit","-qm","init",cwd=repo)
(repo/"app.md").write_text("# app\ntrailing,   \n")           # staged whitespace violation
sh("git","add","app.md",cwd=repo); sh("git","add",".harness-gate",cwd=repo)
F["control"]=gate(repo,"hook")
flow=repo/".harness-gate"/"flow.toml"
orig=flow.read_text()
# neuter the staged-diff step: split into [[steps]] blocks, replace its command with `true`
parts=orig.split("[[steps]]")
out=[]
for i,p in enumerate(parts):
    if 'project.staged-diff-check' in p:
        p=re.sub(r'program = "git"', 'program = "true"', p, count=1)
        p=re.sub(r'args = \[[^\]]*\]', 'args = []', p, count=1, flags=re.S)
    out.append(p)
flow.write_text("[[steps]]".join(out))
sh("git","add",".harness-gate/flow.toml",cwd=repo)
F["neutered_config_check"]=gate(repo,"config","check")
F["neutered_hook"]=gate(repo,"hook")
inv=sorted((repo/".harness-gate"/"reports"/"invocations").glob("*/test_result.json"))
if inv:
    d=json.loads(inv[-1].read_text())
    F["recorded"]={"configuration_digest":d.get("configuration_digest"),
                   "steps":[{"id":s.get("step_id"),"passed":s.get("passed"),"failure_code":s.get("failure_code")} for s in d.get("steps",[])]}
print(json.dumps(F,indent=1,ensure_ascii=False))
pathlib.Path("/home/gem/CodexSymphony/spikes/s4/findings_s4d.json").write_text(json.dumps(F,indent=1,ensure_ascii=False))
print("workdir:",work)

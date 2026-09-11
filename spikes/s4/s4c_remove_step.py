"""S4c: remove the staged-diff step definition entirely from the staged config.
Does `hook` then pass despite a whitespace violation?"""
import json, pathlib, shutil, subprocess, sys, tempfile
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
            "err":next((l.strip()[:160] for l in t.splitlines() if l.strip().startswith(("ERROR","error"))),None),
            "ran":[l.strip() for l in t.splitlines() if l.strip().startswith("[") and ("RUN" in l or "FAIL" in l or "PASS" in l)]}
work=pathlib.Path(tempfile.mkdtemp(prefix="s4c-")); repo=work/"repo"; repo.mkdir()
F={}
sh("git","init","-q","-b","main",cwd=repo); sh("git","config","user.email","a@b.c",cwd=repo); sh("git","config","user.name","a",cwd=repo)
sh(HG,"init","--preset","generic",cwd=repo)
(repo/"app.md").write_text("# app\n")
sh("git","add",".",cwd=repo); sh("git","commit","-qm","init",cwd=repo)
# staged MODIFICATION of a tracked file containing a whitespace violation
(repo/"app.md").write_text("# app\ntrailing,   \n")
sh("git","add","app.md",cwd=repo); sh("git","add",".harness-gate",cwd=repo)
F["control_full_config"]=gate(repo,"hook")
# now delete the whole [[steps]] block for project.staged-diff-check
flow=repo/".harness-gate"/"flow.toml"; txt=flow.read_text()
blocks=txt.split("[[steps]]")
kept=[b for b in blocks if 'project.staged-diff-check' not in b]
print("blocks:",len(blocks),"-> kept:",len(kept))
flow.write_text("[[steps]]".join(kept))
sh("git","add",".harness-gate/flow.toml",cwd=repo)
F["config_check_after_removal"]=gate(repo,"config","check")
F["hook_after_step_removed"]=gate(repo,"hook")
# full bypass attempt: also drop the reference from required_steps
t2=flow.read_text().replace('    "project.staged-diff-check",\n','')
flow.write_text(t2); sh("git","add",".harness-gate/flow.toml",cwd=repo)
F["bypass_config_check"]=gate(repo,"config","check")
F["bypass_hook"]=gate(repo,"hook")
inv=sorted((repo/".harness-gate"/"reports"/"invocations").glob("*/test_result.json"))
if inv:
    d=json.loads(inv[-1].read_text())
    F["recorded"]={"configuration_digest":d.get("configuration_digest"),
                   "steps":[{"id":s.get("step_id"),"passed":s.get("passed")} for s in d.get("steps",[])]}
print(json.dumps(F,indent=1,ensure_ascii=False))
pathlib.Path("/home/gem/CodexSymphony/spikes/s4/findings_s4c.json").write_text(json.dumps(F,indent=1,ensure_ascii=False))
print("workdir:",work)

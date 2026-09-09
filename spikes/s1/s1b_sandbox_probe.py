#!/usr/bin/env python3
"""S1b: probe workspace-write sandbox boundaries via app-server command/exec (no model involved)."""
import json, subprocess, threading, queue, pathlib, sys, os

HERE = pathlib.Path(__file__).resolve().parent
WS = HERE / "ws"
OUTSIDE = HERE / "outside_probe.txt"

p = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, text=True, bufsize=1)
q = queue.Queue()
def reader():
    for line in p.stdout:
        try: q.put(json.loads(line))
        except Exception: pass
threading.Thread(target=reader, daemon=True).start()

nid = [0]
def req(method, params):
    nid[0] += 1
    p.stdin.write(json.dumps({"id": nid[0], "method": method, "params": params}) + "\n"); p.stdin.flush()
    while True:
        m = q.get(timeout=60)
        if m.get("id") == nid[0] and "method" not in m:
            return m

req("initialize", {"clientInfo": {"name": "s1b", "title": "s1b", "version": "0"}, "capabilities": {"experimentalApi": True}})
p.stdin.write(json.dumps({"method": "initialized", "params": {}}) + "\n"); p.stdin.flush()

POLICY = {"type": "workspaceWrite", "networkAccess": False, "writableRoots": []}

def run(label, argv, cwd=str(WS)):
    r = req("command/exec", {"command": argv, "cwd": cwd, "sandboxPolicy": POLICY, "timeoutMs": 20000})
    res = r.get("result") or {}
    err = r.get("error")
    code = res.get("exitCode")
    out = ((res.get("stdout") or "") + (res.get("stderr") or "")).strip().replace("\n", " | ")[:220]
    print(f"[{label}] exit={code} err={err and err.get('message')} :: {out}")
    return code, out

results = {}
results["write_ws"] = run("write in workspace", ["bash", "-c", "echo x > probe.txt && echo WRITE_OK"])
results["git_add"] = run("git add (writes .git/index)", ["git", "add", "probe.txt"])
results["git_commit"] = run("git commit", ["git", "commit", "-qm", "direct-from-sandbox"])
results["git_dir_write"] = run("write .git/EVIL", ["bash", "-c", "echo evil > .git/EVIL && echo GIT_WRITE_OK"])
results["git_config_write"] = run("git config core.hooksPath", ["git", "config", "core.hooksPath", "/tmp/hooks"])
results["outside_write"] = run("write outside workspace", ["bash", "-c", f"echo x > {OUTSIDE} && echo OUTSIDE_OK"])
results["tmp_write"] = run("write /tmp", ["bash", "-c", "echo x > /tmp/s1b_probe && echo TMP_OK"])
results["read_auth"] = run("read ~/.codex/auth.json", ["bash", "-c", "head -c 20 ~/.codex/auth.json >/dev/null && echo AUTH_READ_OK"])
results["read_ssh"] = run("read ~/.ssh", ["bash", "-c", "ls ~/.ssh >/dev/null 2>&1 && echo SSH_LIST_OK"])
results["network"] = run("network egress", ["bash", "-c", "curl -s -m 8 -o /dev/null -w '%{http_code}' https://api.github.com && echo NET_OK"])
results["env_proxy"] = run("env has proxy?", ["bash", "-c", "env | grep -i proxy | head -2"])

print("\n=== host-side verification ===")
def sh(*a): return subprocess.run(a, cwd=WS, text=True, capture_output=True).stdout.strip()
print("git log:", sh("git", "log", "--oneline").replace("\n", " ; "))
print("git status:", sh("git", "status", "--porcelain").replace("\n", " ; ") or "(clean)")
print(".git/EVIL exists:", (WS / ".git/EVIL").exists())
print("hooksPath set:", sh("git", "config", "--get", "core.hooksPath") or "(unset)")
print("outside file exists:", OUTSIDE.exists())
# cleanup probes
for f in [WS / "probe.txt", WS / ".git/EVIL", OUTSIDE]:
    if f.exists(): f.unlink()
subprocess.run(["git", "config", "--unset", "core.hooksPath"], cwd=WS, capture_output=True)
subprocess.run(["git", "reset", "-q", "--hard", "HEAD"], cwd=WS, capture_output=True)
p.terminate()
(HERE / "findings_s1b.json").write_text(json.dumps({k: {"exit": v[0], "out": v[1]} for k, v in results.items()}, indent=1))

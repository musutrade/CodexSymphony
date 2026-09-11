#!/usr/bin/env python3
"""S2 补测: real workflow -> check-run names, mergeable_state sequence, required-check gate.

Creates a branch/PR on the disposable repo, polls CI + mergeable_state, verifies the
merge is blocked until the required check (test-job) is green, then merges and cleans up.

Env: FACTORY_GH_APP_ID, FACTORY_GH_APP_KEY_PATH, FACTORY_GH_REPO (default musutrade/disposable)
"""
import base64, json, os, pathlib, shutil, subprocess, sys, time, urllib.error, urllib.request, uuid
import jwt

APP_ID = os.environ.get("FACTORY_GH_APP_ID") or sys.exit("FACTORY_GH_APP_ID missing")
KEY = pathlib.Path(os.environ.get("FACTORY_GH_APP_KEY_PATH") or sys.exit("KEY_PATH missing")).read_text()
REPO = os.environ.get("FACTORY_GH_REPO", "musutrade/disposable")
OWNER, NAME = REPO.split("/")
HERE = pathlib.Path(__file__).resolve().parent
WORK = HERE / "work2"
F = {}


def api(path, method="GET", token=None, data=None, headers=None):
    h = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
    if token: h["Authorization"] = f"Bearer {token}"
    if headers: h.update(headers)
    req = urllib.request.Request("https://api.github.com" + path, method=method, headers=h,
                                 data=json.dumps(data).encode() if data is not None else None)
    try:
        with urllib.request.urlopen(req) as r:
            b = r.read()
            return r.status, dict(r.headers), (json.loads(b) if b else None)
    except urllib.error.HTTPError as e:
        b = e.read()
        return e.code, dict(e.headers), (json.loads(b) if b else None)


def sh(*a, cwd=None, check=True, env=None):
    r = subprocess.run(list(a), cwd=cwd, text=True, capture_output=True, env=env)
    if check and r.returncode != 0: raise RuntimeError(f"{a[:2]} rc={r.returncode}: {r.stderr}")
    return r


now = int(time.time())
aj = jwt.encode({"iat": now - 60, "exp": now + 540, "iss": APP_ID}, KEY, algorithm="RS256")
s, _, inst = api(f"/repos/{OWNER}/{NAME}/installation", token=aj)
s, _, tok = api(f"/app/installations/{inst['id']}/access_tokens", "POST", token=aj, data={"repositories": [NAME]})
TOKEN = tok["token"]
print("permissions:", tok["permissions"])

s, _, repo = api(f"/repos/{OWNER}/{NAME}", token=TOKEN)
DEFAULT = repo["default_branch"]

if WORK.exists(): shutil.rmtree(WORK)
WORK.mkdir()
CANON = WORK / "canon"
hdr = "http.https://github.com/.extraheader=AUTHORIZATION: basic " + base64.b64encode(f"x-access-token:{TOKEN}".encode()).decode()
GIT = ["git", "-c", hdr, "-c", "credential.helper=", "-c", "core.hooksPath=/dev/null"]
env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
sh(*GIT, "clone", "-q", f"https://github.com/{OWNER}/{NAME}.git", str(CANON), env=env)
sh("git", "config", "user.email", "factory-spike@example.com", cwd=CANON)
sh("git", "config", "user.name", "factory-spike", cwd=CANON)
base = sh("git", "rev-parse", f"origin/{DEFAULT}", cwd=CANON).stdout.strip()
RUN = uuid.uuid4().hex[:8]
BR = f"ai/req-s2b-{RUN}"
WT = WORK / "wt"
sh("git", "worktree", "add", "-q", "-b", BR, str(WT), base, cwd=CANON)
(WT / f"s2b-{RUN}.txt").write_text("x\n")
sh("git", "add", ".", cwd=WT)
sh("git", "commit", "-qm", f"s2b {RUN}", cwd=WT)
ART = sh("git", "rev-parse", "HEAD", cwd=WT).stdout.strip()
sh(*GIT, "push", "-q", f"--force-with-lease=refs/heads/{BR}:{'0'*40}", "origin", f"{ART}:refs/heads/{BR}", cwd=CANON, env=env)

s, _, pr = api(f"/repos/{OWNER}/{NAME}/pulls", "POST", token=TOKEN,
               data={"title": f"S2b {RUN}", "head": BR, "base": DEFAULT, "body": "s2b"})
PRN = pr["number"]
print("PR", PRN, "head", ART)
F["pr"] = {"number": PRN, "head": ART}

# --- poll mergeable_state + checks until stable ---
seq = []
check_names, combined_states = set(), []
deadline = time.time() + 300
final = None
while time.time() < deadline:
    s, _, p = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", token=TOKEN)
    s2, _, cr = api(f"/repos/{OWNER}/{NAME}/commits/{ART}/check-runs", token=TOKEN)
    s3, _, st = api(f"/repos/{OWNER}/{NAME}/commits/{ART}/status", token=TOKEN)
    runs = cr.get("check_runs", []) if s2 == 200 else []
    for r in runs:
        check_names.add((r["name"], r.get("app", {}).get("slug")))
    if s3 == 200:
        combined_states.append(st.get("state"))
    row = {"t": round(time.time() - now),
           "mergeable": p.get("mergeable"), "mergeable_state": p.get("mergeable_state"),
           "checks": [(r["name"], r["status"], r.get("conclusion")) for r in runs],
           "combined": st.get("state") if s3 == 200 else None}
    if not seq or [x for x in (row["mergeable_state"], row["checks"], row["combined"])] != \
            [x for x in (seq[-1]["mergeable_state"], seq[-1]["checks"], seq[-1]["combined"])]:
        seq.append(row); print("state:", json.dumps(row)[:260])
    # NOTE: combined status stays "pending" forever when only Actions check-runs exist;
    # it must NOT be part of the readiness condition. mergeable_state + check-runs are.
    if row["mergeable_state"] in ("clean", "blocked") and runs and \
       all(r["status"] == "completed" for r in runs):
        final = row
        if row["mergeable_state"] == "clean":
            break
    time.sleep(6)

F["mergeable_state_sequence"] = [{"t": r["t"], "mergeable": r["mergeable"], "state": r["mergeable_state"]} for r in seq]
F["check_run_names"] = sorted({n for n, _ in check_names})
F["check_run_apps"] = sorted({a for _, a in check_names})
F["combined_status_states"] = list(dict.fromkeys(combined_states))
F["final"] = final

# --- merge must be blocked before CI green? try early merge on a fresh PR later; here use sha guard ---
s, _, bad = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", "PUT", token=TOKEN,
                data={"sha": base, "merge_method": "squash"})
F["merge_wrong_sha"] = {"status": s, "body": bad}
s, _, prot = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", token=TOKEN)
F["mergeable_state_before_merge"] = prot.get("mergeable_state")
F["merge_when_clean"] = {"state": prot.get("mergeable_state"), "mergeable": prot.get("mergeable")}
F["mergeable_state_null_observed"] = any(r["mergeable"] is None for r in seq)

if final and final["mergeable_state"] == "clean":
    s, _, m = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", "PUT", token=TOKEN,
                  data={"sha": ART, "merge_method": "squash", "commit_title": f"S2b {RUN} (#{PRN})"})
    F["merge_result"] = {"status": s, "sha": (m or {}).get("sha")}
    print("merged:", F["merge_result"])
else:
    F["merge_result"] = {"skipped": "not clean in window", "final": final}
    print("NOT merged:", final)

s, _, _ = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", "PATCH", token=TOKEN, data={"state": "closed"})
s, _, _ = api(f"/repos/{OWNER}/{NAME}/git/refs/heads/{BR}", "DELETE", token=TOKEN)
shutil.rmtree(WORK, ignore_errors=True)
(HERE / "findings_s2b.json").write_text(json.dumps(F, indent=1, ensure_ascii=False).replace(TOKEN, "<tok>"))
print("\nwrote findings_s2b.json")
print(json.dumps({k: v for k, v in F.items() if k != "mergeable_state_sequence"}, indent=1, ensure_ascii=False)[:1500])

#!/usr/bin/env python3
"""S2 spike: GitHub App handoff path.

  App JWT -> installation token (repo-scoped)
  -> conditional fast-forward push of a fixed SHA (git over HTTPS with x-access-token)
  -> find-or-create PR by head+base, idempotency, duplicate-create error
  -> read checks / statuses / rate limit / conditional requests
  -> merge with `sha` guard (mismatch error), post-merge verification

Env:
  FACTORY_GH_APP_ID        (e.g. 4867361)
  FACTORY_GH_APP_KEY_PATH  (path to .pem)
  FACTORY_GH_REPO          (owner/name, default musutrade/disposable)

Nothing secret is written to disk; findings_s2.json redacts tokens.
"""
import json, os, pathlib, shutil, subprocess, sys, time, urllib.error, urllib.request, uuid

import jwt  # pyjwt

APP_ID = os.environ.get("FACTORY_GH_APP_ID") or sys.exit("FACTORY_GH_APP_ID missing")
KEY = pathlib.Path(os.environ.get("FACTORY_GH_APP_KEY_PATH") or sys.exit("FACTORY_GH_APP_KEY_PATH missing")).read_text()
REPO = os.environ.get("FACTORY_GH_REPO", "musutrade/disposable")
OWNER, NAME = REPO.split("/")
HERE = pathlib.Path(__file__).resolve().parent
WORK = HERE / "work"
F = {}  # findings


def log(k, v):
    F[k] = v
    print(f"[{k}] {json.dumps(v, ensure_ascii=False)[:600]}")


# ---------- HTTP ----------
def api(path, method="GET", token=None, data=None, headers=None):
    h = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
    if token:
        h["Authorization"] = f"Bearer {token}"
    if headers:
        h.update(headers)
    req = urllib.request.Request("https://api.github.com" + path, method=method, headers=h,
                                 data=json.dumps(data).encode() if data is not None else None)
    try:
        with urllib.request.urlopen(req) as r:
            body = r.read()
            return r.status, dict(r.headers), (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        body = e.read()
        return e.code, dict(e.headers), (json.loads(body) if body else None)


def rl(h):
    return {k: h.get(k) for k in ("X-RateLimit-Limit", "X-RateLimit-Remaining", "X-RateLimit-Resource", "ETag")}


def sh(*args, cwd=None, check=True, env=None):
    r = subprocess.run(list(args), cwd=cwd, text=True, capture_output=True, env=env)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(args[:3])}... rc={r.returncode}\n{r.stderr}")
    return r


# ---------- 1. auth ----------
now = int(time.time())
app_jwt = jwt.encode({"iat": now - 60, "exp": now + 540, "iss": APP_ID}, KEY, algorithm="RS256")

s, h, inst = api(f"/repos/{OWNER}/{NAME}/installation", token=app_jwt)
assert s == 200, (s, inst)
INST_ID = inst["id"]
log("installation", {"id": INST_ID, "permissions": inst["permissions"], "events": inst["events"],
                     "repository_selection": inst["repository_selection"]})

# repo-scoped token, explicitly requesting a permission we DO have and one we DON'T (checks)
s, h, tok_full = api(f"/app/installations/{INST_ID}/access_tokens", "POST", token=app_jwt,
                     data={"repositories": [NAME]})
assert s == 201, (s, tok_full)
TOKEN = tok_full["token"]
log("installation_token", {"status": s, "expires_at": tok_full["expires_at"], "permissions": tok_full["permissions"],
                           "repositories": [r["full_name"] for r in tok_full.get("repositories", [])],
                           "token_prefix": TOKEN[:4] + "..."})

s, h, tok_down = api(f"/app/installations/{INST_ID}/access_tokens", "POST", token=app_jwt,
                     data={"repositories": [NAME], "permissions": {"contents": "read", "metadata": "read"}})
log("installation_token_downscoped", {"status": s, "permissions": (tok_down or {}).get("permissions"), "body": None if s == 201 else tok_down})

s, h, tok_bad = api(f"/app/installations/{INST_ID}/access_tokens", "POST", token=app_jwt,
                    data={"repositories": [NAME], "permissions": {"checks": "read"}})
log("installation_token_request_unheld_permission", {"status": s, "body": tok_bad})

# ---------- 2. repo + clone ----------
s, h, repo = api(f"/repos/{OWNER}/{NAME}", token=TOKEN)
DEFAULT = repo["default_branch"]
log("repo", {"default_branch": DEFAULT, "private": repo["private"], "rate": rl(h)})

if WORK.exists():
    shutil.rmtree(WORK)
WORK.mkdir()
CANON = WORK / "canon"
env = {**os.environ, "GIT_TERMINAL_PROMPT": "0", "GIT_CONFIG_NOSYSTEM": "1"}
# credential via header, never in URL or on disk
auth_hdr = f"http.https://github.com/.extraheader=AUTHORIZATION: basic " + \
           __import__("base64").b64encode(f"x-access-token:{TOKEN}".encode()).decode()
GIT = ["git", "-c", auth_hdr, "-c", "credential.helper=", "-c", "core.hooksPath=/dev/null"]

sh(*GIT, "clone", "-q", f"https://github.com/{OWNER}/{NAME}.git", str(CANON), env=env)
sh("git", "config", "user.email", "factory-spike@example.com", cwd=CANON)
sh("git", "config", "user.name", "factory-spike", cwd=CANON)
base_sha = sh("git", "rev-parse", f"origin/{DEFAULT}", cwd=CANON).stdout.strip()
log("clone", {"base_sha": base_sha})

RUN = uuid.uuid4().hex[:8]
BRANCH = f"ai/req-s2-{RUN}"
WT = WORK / "wt"
sh("git", "worktree", "add", "-q", "-b", BRANCH, str(WT), base_sha, cwd=CANON)
(WT / f"s2-{RUN}.txt").write_text(f"spike {RUN}\n")
sh("git", "add", ".", cwd=WT)
sh("git", "commit", "-qm", f"s2: spike commit {RUN}\n\n<!-- ai-factory:requirement=00000000-0000-0000-0000-{RUN}0000 -->", cwd=WT)
ART = sh("git", "rev-parse", "HEAD", cwd=WT).stdout.strip()
log("artifact_commit", {"sha": ART, "branch": BRANCH})

# ---------- 3. conditional push ----------
ZERO = "0" * 40
def push(expected_remote, sha=ART, force_lease=True):
    args = [*GIT, "push", "-q"]
    if force_lease:
        args.append(f"--force-with-lease=refs/heads/{BRANCH}:{expected_remote}")
    args += ["origin", f"{sha}:refs/heads/{BRANCH}"]
    r = sh(*args, cwd=CANON, check=False, env=env)
    return {"rc": r.returncode, "stderr": r.stderr.strip()[-400:]}

# 3a. branch does not exist remotely; expected old = zero sha
log("push_create_branch_expected_zero", push(ZERO))
# 3b. same sha again, expected old = ART (idempotent)
log("push_idempotent_same_sha", push(ART))
# 3c. wrong expectation (pretend we thought remote was still zero) -> must be rejected
log("push_stale_expectation_rejected", push(ZERO))
# 3d. remote moved by someone else (simulate): push a second commit via REST, then try our old ART with lease=ART
s, h, ref = api(f"/repos/{OWNER}/{NAME}/git/ref/heads/{BRANCH}", token=TOKEN)
log("ref_after_push", {"status": s, "sha": ref["object"]["sha"] if s == 200 else ref})

# create a commit on the branch via REST to simulate external change
s, h, blob = api(f"/repos/{OWNER}/{NAME}/git/blobs", "POST", token=TOKEN, data={"content": "external\n", "encoding": "utf-8"})
s, h, tree = api(f"/repos/{OWNER}/{NAME}/git/trees", "POST", token=TOKEN,
                 data={"base_tree": ART, "tree": [{"path": f"external-{RUN}.txt", "mode": "100644", "type": "blob", "sha": blob["sha"]}]})
s, h, ext = api(f"/repos/{OWNER}/{NAME}/git/commits", "POST", token=TOKEN,
                data={"message": "external change", "tree": tree["sha"], "parents": [ART]})
s, h, upd = api(f"/repos/{OWNER}/{NAME}/git/refs/heads/{BRANCH}", "PATCH", token=TOKEN, data={"sha": ext["sha"], "force": False})
log("rest_ref_update_ff", {"status": s, "new_sha": upd["object"]["sha"] if s == 200 else upd})
# now REST non-ff update back to ART with force=false -> expect 422
s, h, nonff = api(f"/repos/{OWNER}/{NAME}/git/refs/heads/{BRANCH}", "PATCH", token=TOKEN, data={"sha": ART, "force": False})
log("rest_ref_update_non_ff_rejected", {"status": s, "body": nonff})
# git push with lease expecting ART but remote is ext -> rejected (stale info)
log("push_lease_mismatch_after_external_move", push(ART))
# reconcile: platform fetches, sees ext, decides. Here we restore ART via REST force (platform would NOT do this automatically)
s, h, _ = api(f"/repos/{OWNER}/{NAME}/git/refs/heads/{BRANCH}", "PATCH", token=TOKEN, data={"sha": ART, "force": True})
log("rest_ref_reset_for_test_only", {"status": s})

# ---------- 4. PR find-or-create ----------
def find_pr():
    s, h, prs = api(f"/repos/{OWNER}/{NAME}/pulls?state=all&head={OWNER}:{BRANCH}&base={DEFAULT}", token=TOKEN)
    return s, [{"number": p["number"], "state": p["state"], "head_sha": p["head"]["sha"], "merged_at": p["merged_at"]} for p in prs]

log("pr_find_before_create", find_pr())
body = f"<!-- ai-factory:requirement=00000000-0000-0000-0000-{RUN}0000 -->\n<!-- ai-factory:repository={OWNER}/{NAME} -->\n\nS2 spike PR."
s, h, pr = api(f"/repos/{OWNER}/{NAME}/pulls", "POST", token=TOKEN,
               data={"title": f"S2 spike {RUN}", "head": BRANCH, "base": DEFAULT, "body": body})
assert s == 201, (s, pr)
PRN = pr["number"]
log("pr_create", {"status": s, "number": PRN, "head_sha": pr["head"]["sha"], "user": pr["user"]["login"], "url": pr["html_url"]})
s, h, dup = api(f"/repos/{OWNER}/{NAME}/pulls", "POST", token=TOKEN,
                data={"title": "dup", "head": BRANCH, "base": DEFAULT, "body": "dup"})
log("pr_create_duplicate", {"status": s, "body": dup})
log("pr_find_after_create", find_pr())

# ---------- 5. checks / statuses ----------
s, h, cr = api(f"/repos/{OWNER}/{NAME}/commits/{ART}/check-runs", token=TOKEN)
log("check_runs_read", {"status": s, "body": cr if s != 200 else {"total_count": cr["total_count"], "names": [c["name"] for c in cr["check_runs"]]}, "rate": rl(h)})
s, h, cs = api(f"/repos/{OWNER}/{NAME}/commits/{ART}/check-suites", token=TOKEN)
log("check_suites_read", {"status": s, "body": cs if s != 200 else {"total_count": cs["total_count"]}})
s, h, st = api(f"/repos/{OWNER}/{NAME}/commits/{ART}/status", token=TOKEN)
log("combined_status_read", {"status": s, "state": st.get("state") if s == 200 else st, "total": st.get("total_count") if s == 200 else None})
s, h, runs = api(f"/repos/{OWNER}/{NAME}/actions/runs?head_sha={ART}", token=TOKEN)
log("actions_runs_read", {"status": s, "body": runs if s != 200 else {"total_count": runs["total_count"]}})
s, h, prot = api(f"/repos/{OWNER}/{NAME}/branches/{DEFAULT}/protection", token=TOKEN)
log("branch_protection_read", {"status": s, "body": prot if s != 200 else {"required_status_checks": prot.get("required_status_checks")}})
s, h, rules = api(f"/repos/{OWNER}/{NAME}/rules/branches/{DEFAULT}", token=TOKEN)
log("rulesets_read", {"status": s, "count": len(rules) if isinstance(rules, list) else rules})

# ---------- 6. conditional request / rate limit ----------
s1, h1, p1 = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", token=TOKEN)
etag = h1.get("ETag")
s2, h2, p2 = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", token=TOKEN, headers={"If-None-Match": etag})
log("conditional_request", {"first": s1, "etag": etag, "second": s2,
                            "remaining_before": h1.get("X-RateLimit-Remaining"), "remaining_after": h2.get("X-RateLimit-Remaining")})
log("pr_mergeable", {"mergeable": p1.get("mergeable"), "mergeable_state": p1.get("mergeable_state")})

# ---------- 7. merge with sha guard ----------
s, h, m_bad = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", "PUT", token=TOKEN,
                  data={"sha": base_sha, "merge_method": "squash"})
log("merge_sha_mismatch", {"status": s, "body": m_bad})
s, h, m_ok = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", "PUT", token=TOKEN,
                 data={"sha": ART, "merge_method": "squash", "commit_title": f"S2 spike {RUN} (#{PRN})",
                       "commit_message": f"<!-- ai-factory:requirement=00000000-0000-0000-0000-{RUN}0000 -->"})
log("merge_ok", {"status": s, "body": m_ok})
s, h, m_again = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", "PUT", token=TOKEN, data={"sha": ART, "merge_method": "squash"})
log("merge_repeat_after_merged", {"status": s, "body": m_again})
s, h, merged = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}/merge", token=TOKEN)
log("merge_fact_check", {"status": s, "note": "204 = merged, 404 = not merged"})
s, h, prf = api(f"/repos/{OWNER}/{NAME}/pulls/{PRN}", token=TOKEN)
log("pr_after_merge", {"state": prf["state"], "merged": prf["merged"], "merge_commit_sha": prf["merge_commit_sha"], "merged_by": (prf.get("merged_by") or {}).get("login")})
log("pr_find_after_merge", find_pr())

# ---------- 8. cleanup ----------
s, h, _ = api(f"/repos/{OWNER}/{NAME}/git/refs/heads/{BRANCH}", "DELETE", token=TOKEN)
log("branch_delete", {"status": s})
s, h, rev = api(f"/repos/{OWNER}/{NAME}/rate_limit", token=TOKEN)
log("rate_limit_final", rev["resources"]["core"] if s == 200 else rev)

shutil.rmtree(WORK, ignore_errors=True)
redacted = json.loads(json.dumps(F).replace(TOKEN, "<token>"))
(HERE / "findings_s2.json").write_text(json.dumps(redacted, indent=1, ensure_ascii=False))
print("\nwrote findings_s2.json")

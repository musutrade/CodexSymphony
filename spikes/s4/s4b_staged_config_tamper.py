#!/usr/bin/env python3
"""S4b: does harness-gate 'hook' accept a *weakened but schema-valid* staged config?

Plan 12.6.1 claims an agent must not be able to make the gate pass by editing the gate's own
config. This script tests that claim against harness-gate 0.3.7 by, in a throwaway git repo:

  1. baseline: valid config, a file with a real secrets-scan hit -> hook must FAIL
  2. weakened: remove the secret-scan/whitespace steps from flow.toml and stage that config
     -> does hook still FAIL, or does it PASS (i.e. the agent can disable its own gate)?

Usage:  python3 s4b_staged_config_tamper.py /path/to/harness-gate
"""
import json, os, pathlib, shutil, subprocess, sys, tempfile

HG = sys.argv[1] if len(sys.argv) > 1 else shutil.which("harness-gate") or sys.exit("pass the harness-gate path")
F = {}


def sh(*a, cwd=None, check=True):
    r = subprocess.run(list(a), cwd=cwd, text=True, capture_output=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"{a[:3]} rc={r.returncode}\n{r.stderr}")
    return r


def gate(repo, *args):
    r = subprocess.run([HG, "--project-root", str(repo), *args], text=True, capture_output=True)
    text = (r.stdout + r.stderr)
    return {"rc": r.returncode, "summary": next((l.strip() for l in text.splitlines() if l.startswith("TEST_SUMMARY")), None),
            "tail": text.strip()[-400:]}


work = pathlib.Path(tempfile.mkdtemp(prefix="s4b-"))
repo = work / "repo"
repo.mkdir()
sh("git", "init", "-q", "-b", "main", cwd=repo)
sh("git", "config", "user.email", "s4b@example.com", cwd=repo)
sh("git", "config", "user.name", "s4b", cwd=repo)
sh(HG, "init", "--preset", "generic", cwd=repo)
(repo / "app.md").write_text("# app\n")
sh("git", "add", ".", cwd=repo)
sh("git", "commit", "-qm", "init", cwd=repo)
sh("git", "add", ".harness-gate", cwd=repo)          # config must be staged for `hook`

# --- 1. baseline: a real-looking credential in the staged file -> hook must FAIL
(repo / "leak.md").write_text('key = "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8"\n')
sh("git", "add", "leak.md", cwd=repo)
F["baseline_with_secret"] = gate(repo, "hook")
F["baseline_config_digest"] = sh("sha256sum", ".harness-gate/flow.toml", cwd=repo).stdout.split()[0]

# Remove the secret so it stops masking the result: we want to see whether the
# *configurable* steps can be disabled by editing the staged config.
sh("git", "rm", "-q", "--cached", "leak.md", cwd=repo)
(repo / "leak.md").unlink()
# a whitespace violation: this IS governed by a configurable step
(repo / "ws.md").write_text("trailing space,   \n")
sh("git", "add", "ws.md", cwd=repo)
F["unweakened_with_whitespace_violation"] = gate(repo, "hook")

# --- 2. weakened but schema-valid staged config: drop the staged-diff step
flow = repo / ".harness-gate" / "flow.toml"
original = flow.read_text()
weakened = original.replace('    "project.staged-diff-check",\n', "")
assert weakened != original, f"could not weaken the preset config:\n{original[:400]}"
flow.write_text(weakened)
sh("git", "add", ".harness-gate/flow.toml", cwd=repo)
F["weakened_config_digest"] = sh("sha256sum", ".harness-gate/flow.toml", cwd=repo).stdout.split()[0]
F["config_digest_changed"] = F["baseline_config_digest"] != F["weakened_config_digest"]
F["weakened_config_still_schema_valid"] = gate(repo, "config", "check")
F["weakened_with_whitespace_violation"] = gate(repo, "hook")

# --- 4. what does the invocation record say about which config it used?
inv = sorted((repo / ".harness-gate" / "reports" / "invocations").glob("*/test_result.json"))
if inv:
    d = json.loads(inv[-1].read_text())
    F["recorded"] = {k: d.get(k) for k in
                     ("executor_version", "input_mode", "source_identity", "configuration_digest", "profile")}
    F["recorded_steps"] = [{"step_id": s.get("step_id"), "passed": s.get("passed"), "failure_code": s.get("failure_code")}
                           for s in d.get("steps", [])]

print(json.dumps(F, indent=1, ensure_ascii=False))
pathlib.Path(__file__).with_name("findings_s4b.json").write_text(json.dumps(F, indent=1, ensure_ascii=False))
print("\nworkdir kept for inspection:", work)

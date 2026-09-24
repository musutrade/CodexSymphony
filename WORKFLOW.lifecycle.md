---
tracker:
  kind: github
  provider:
    repo: musutrade/CodexSymphony
    token: $GITHUB_TOKEN
    symphony_lifecycle: true
    publication_guard: /home/gem/.local/share/codexsymphony/publication/guard
    lifecycle_base_branch: main
    lifecycle_check_app: github-actions
    lifecycle_poll_interval_ms: 60000
    lifecycle_max_repair_attempts: 1
    lifecycle_required_checks:
      - Harness-Gate
  required_labels:
    - symphony-ready
  active_states:
    - open
  terminal_states:
    - closed
polling:
  interval_ms: 15000
observability:
  dashboard_enabled: false
workspace:
  root: $CODEXSYMPHONY_WORKSPACE_ROOT
hooks:
  timeout_ms: 300000
  before_remove_required: true
  before_remove: |
    python3 /home/gem/.local/share/codexsymphony/symphony/preserve_workspace.py "$PWD" --archive /home/gem/.local/share/codexsymphony/symphony/retained-evidence
  after_run: |
    python3 /home/gem/.local/share/codexsymphony/symphony/preserve_workspace.py "$PWD" --archive /home/gem/.local/share/codexsymphony/symphony/retained-evidence
  after_create: |
    set -eu
    timeout --kill-after=5s 280s git clone --depth 1 https://github.com/musutrade/CodexSymphony.git .
    git config --local core.hooksPath /dev/null
    git config --local credential.helper ''
    printf '\n.symphony-handoff.json\n.agent-cargo/\n.agent-tmp/\n' >> .git/info/exclude
  before_run: |
    set -eu
    /usr/bin/python3 /home/gem/.local/share/codexsymphony/symphony/check_deployment.py
    eval "$(python3 /home/gem/.local/share/codexsymphony/symphony/environment_contract.py shell)"
    python3 /home/gem/.local/share/codexsymphony/symphony/environment_contract.py check
    python3 tools/gate.py config check
    mkdir -p target
    test -w target
    python3 /home/gem/.local/share/codexsymphony/symphony/provision_issue_environment.py "$PWD"
agent:
  serial_delivery: true
  max_concurrent_agents: 1
  max_turns: 40
  max_retry_backoff_ms: 300000
  max_total_tokens: 100000000
  max_runtime_ms: 14400000
  max_consecutive_errors: 6
  max_run_attempts: 3
  max_infrastructure_attempts: 3
server:
  operator_token_env: SYMPHONY_OPERATOR_TOKEN
codex:
  command: >-
    /home/gem/.local/share/codexsymphony/symphony/codex-trusted
    --config 'model_provider="openai"'
    --config 'model="gpt-6-astra"'
    --config 'model_reasoning_effort="low"'
    --config 'features.goals=false'
    --config 'features.apps=false'
    --config 'tool_output_token_limit=2000'
    app-server
  approval_policy: never
  thread_sandbox: danger-full-access
  turn_sandbox_policy:
    type: dangerFullAccess
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000
---

You are implementing GitHub Issue {{ issue.identifier }} (ID {{ issue.id }})
in musutrade/CodexSymphony.
Title: {{ issue.title }}
State: {{ issue.state }}

{% if issue.description %}
{{ issue.description }}
{% endif %}

## Scope and source of truth

Work only in the assigned Symphony workspace. Read README.md,
docs/architecture-boundaries.md, and the sections of
Personal_AI_Software_Factory_综合方案.md relevant to this Issue. Chapters 23 and
24 define phase scope and acceptance. Follow applicable AGENTS.md instructions.
Implement only this Issue's acceptance criteria and declared dependencies.
Missing requirements or dependencies are blockers, not permission to invent scope.

This workflow drives the existing local Elixir Symphony to BUILD CodexSymphony.
Its GitHub Issue lifecycle is distinct from the Rust platform being implemented.
The product must preserve Requirement business truth, AgentRun execution facts,
GitHub/CI observations, and identity-bound validation evidence independently.
Do not implement Gate PASS -> Done or validation FAIL -> AgentRun failed.

The product Phase 0a uses custom validation and ends at Submitted/PR. This
repository itself requires Harness-Gate now (see .harness-gate/QUALITY.md).
Its own runtime must
use a worktree per Run, matching app-server cwd, ordinary local Git and
platform-owned Git delivery. Do not transplant this development controller's
development handoff protocol into the product. Cloudflare/Bark belong to 0b;
independent executor UID and the newly pinned Harness-Gate integration to 0c.

## Provisioned serial-development environment

The host before_run hook provisions an independent disposable
PostgreSQL 16 test fixture, a synthetic persistent fixture, offline npm cache and
read-only browser/source resources per assigned Issue. Read `.agent-env/README.md`
before probing services. Previous preparation reports may describe an earlier
unprovisioned environment; confirm the current fixture using the commands below.

Use `python3 /opt/symphony-env/run.py COMMAND ...` to inject the supplied
project database URLs; this wrapper executes the ordinary command directly.
For the required full Rust suite, use `python3 /opt/symphony-env/workspace_tests.py`.
It recreates only the disposable test fixture, checks its resource policy,
sets a fresh short temporary directory and runs the same locked Cargo command
with serial Rust test scheduling. Preserve its output and exit status.
Run Cargo tests, including the real Runtime integration, in this same trusted
development environment. No reviewed binary installation, source manifest,
backend_tests.py or runtime_product_acceptance.py receipt is required.
Do not reintroduce per-test host execution endpoints or managed-network probes.

## Preparation and recovery

Before coding, check declared tools, writable build paths and required service
connectivity in the actual development environment. Run the relevant real smoke
tests. Missing dependencies are environment failures, not code repair attempts.

Use the injected `github_api` tool for authorized GitHub operations. Its service
holds credentials outside this environment. Local Git is writable and usable.
Do not read or copy service credentials. Formal Gate signing remains in the
independent verifier; ordinary local tests neither require nor produce signatures.

On retry, inspect current branch, working tree, existing commits and saved
evidence first. Preserve uncommitted and untracked implementation. Never reset
or clean away progress, force-push, or restart from a fresh baseline just because
the previous run ended. Read bounded evidence summaries before full logs.
If blocked, report: confirmed facts versus hypotheses, saved progress, attempted
actions, the specific next action, the machine-checkable recovery condition,
and the stage to resume. Do not claim a blocker was resolved without evidence.

## Implementation and validation

Keep Rust domain logic independent of runtime, persistence and GitHub adapters.
Use the Angular skills when implementing the frontend. Phase 0a uses the minimal
Material UI specified in chapter 23. Avoid implementing later phases implicitly.

Run the Issue's acceptance commands and the checks relevant to changed code.
Once a root Cargo workspace exists, Rust changes require cargo fmt --all -- --check,
the full `cargo test --workspace --locked` through the prepared fixture entry,
and cargo clippy --workspace --all-targets --locked
-- -D warnings, unless a reviewed repository policy defines a more precise scope.
Frontend changes use the checked-in package scripts and lockfile. Do not fabricate
commands, passing results, coverage, or evidence for absent code and infrastructure.
An applicable acceptance check that cannot run is incomplete, not N/A.

Do not run the real GitHub/cloud/notification spikes by default. Tests with
external side effects require task authorization and the designated test setup.
Keep production secrets out of this repository. Do not change CI, required checks,
this workflow or acceptance policy merely to make the current task pass.
Before publishing, finish the evidence manifest and all source edits, then call
`local_gate` with `{"action":"start"}`. This starts the same complete installed
Gate used by CI without publishing a commit. Read `local_gate` with
`{"action":"status"}` after a bounded wait (at least 60 seconds); ordinary local
checks may be used while developing, but only this complete PASS permits delivery.
A failed result includes the retained log directory; fix the source and start a
new validation. Do not publish to discover quality failures. The host receipt
binds the exact Git tree (including executable modes and deletions), environment
fingerprint and approved Gate implementation. Any source/environment change
invalidates it. The GitHub tool rejects unvalidated refs/PRs and alternate write
paths such as the contents API. Do not edit source or evidence after PASS.
For this repository run `python3 tools/gate.py config check` as a diagnostic. The separately installed trusted host
runs `python3 tools/gate.py verify --profile ci --all` on the exact published
commit; GitHub Actions waits for its App-authenticated result. Agent workspaces
do not receive signing keys or reusable trusted runtime requests. CRAP <= 10 is
required; missing/unsupported measurements remain blockers. Do not delete quality
configuration, lower requiredness, substitute N/A, or reuse another source's
measurement evidence. Hook is partial assurance and cannot replace full CI.
Read .harness-gate/QUALITY.md and docs/remote-gate.md for provisioning and evidence.

Prefer focused reads and bounded output; preserve full evidence in artifacts.
Before handoff, create `.symphony-evidence.json` using schema `symphony-evidence/v1`
and `files: [{"path":"artifacts/...","sha256":"<exact SHA-256>"}]`. Include all
required evidence declared by this task, using canonical workspace-relative paths.
An empty list requires an explicit `empty_reason`; missing evidence is not empty.
Include the manifest path in the validation summary. The host `after_run` hook
copies the declared evidence outside the workspace and logs its receipt; the Agent
does not have access to the host script or archive and must not claim a copy occurred.
Cleanup independently repeats the check; missing files, checksum errors or timeouts
retain the workspace. An `after_run` failure does not authorize deletion.
Stop testing when the required checks pass unless new changes or failures justify
more testing. Do not use goal tools, spawn additional agents, or expand budgets.
The controller owns attempts, cumulative budgets and CI waiting.

## Delivery using the existing local controller

The controller creates symphony/GH-<number> from a fresh origin/main for new work.
Keep that branch and preserve dirty work on retries. Publish validated workspace
changes through `github_api`, executed by the Symphony host:

1. Confirm `local_gate` status is PASS for the current tree. Read the expected remote branch/base commit and tree. Establish the intended
   parent from the controller's baseline or the last confirmed publication on
   this same Issue. Unexpected remote movement requires reconciliation first.
2. Use Git Data API blobs/trees/commits to represent the exact validated source:
   include additions, modifications, deletions and file modes. Preserve unchanged
   base-tree entries. Exclude generated outputs, secrets and the handoff file.
   Use base64 blob content for bytes that cannot be represented safely as text.
3. Create the Issue branch ref if absent, or update it with `force: false`.
   Use the expected parent; do not overwrite divergent work. On a lost response,
   read the remote identity before retrying rather than creating duplicate work.
4. Find or create the same-repository PR targeting main through `github_api`.
   Read back both ref and PR head and verify the remote tree matches the validated
   workspace changes. Local HEAD may remain the baseline: it is NOT necessarily
   the published SHA. Record the actual host-returned and read-back remote SHA.
5. Describe the problem, resulting behavior, exact validation and remaining limits.
   Do not use automatic Issue-closing keywords; the controller closes after
   confirmed delivery. Do not claim product-wide acceptance from one merged Issue.

After confirmed publication, write the UNTRACKED workspace-root
.symphony-handoff.json as a regular file, at most 16 KiB. Use the actual PR
identity and full remote SHA; never emit the example as a real declaration.

```json
{
  "issue_id": "{{ issue.id }}",
  "repo": "musutrade/CodexSymphony",
  "number": 123,
  "branch": "symphony/GH-REPLACE_WITH_ISSUE_NUMBER",
  "head_sha": "REPLACE_WITH_FULL_PUBLISHED_COMMIT_SHA",
  "validation_summary": "Actual commands, outcomes, evidence references and limitations."
}
```

Declare after applicable local acceptance is satisfied and the exact PR head is
confirmed. CI acceptance remains pending until the
controller observes both required protected checks; do not claim them locally.
End the turn immediately afterward with "submitted; CI pending". Do not poll CI,
merge, close Issues or edit the host handoff journal. The controller handles
those actions. On an explicitly dispatched repair, reuse the same PR and branch,
validate the repair, publish through the host tool using the last confirmed
remote head as parent, replace the declaration and stop again.

### Explicit blocked handoff

If a required external capability is unavailable and you have retained a concrete
reproduction, stop immediately by writing `.symphony-handoff.json` with exactly:

```json
{"status":"blocked","issue_id":"<assigned numeric id>","repo":"musutrade/CodexSymphony","reason":"<confirmed cause>","evidence":"<retained evidence path>","resume_condition":"<machine-checkable recovery>"}
```

This requests a durable pause, not PR completion. Do not include PR fields, repeat
unchanged probes, or keep consuming continuation turns. The controller reads it
after the turn, records the cause, and will not retry until operator recovery.
The authenticated host recovery API archives this declaration only after recovery is durably recorded; never edit the host journal.
Ordinary code/test failures within your ability to fix are not external blockers.


### External host operations and bounded recovery

Use `waiting_external` when the next step belongs to an already authorized host
operation. Do not report it as a code failure or ask again for authorization that
this workflow or the Issue already supplies. Save progress first, write exactly:

```json
{"status":"waiting_external","issue_id":"<assigned numeric id>","repo":"musutrade/CodexSymphony","operation":"<registered host operation>","request_id":"<unique stage request>","reason":"<confirmed prerequisite>","evidence":"<retained non-secret evidence>","resume_condition":"<precise checks required before continuation>"}
```

Then stop the turn. The controller retains the queue and usage; the host bridge
runs a matching pinned authorization and records a receipt before resuming this
same Issue. Never include shell commands or secrets in the declaration. The
read-only `product.identity_preflight` operation is preauthorized for the current
A01/A13 deployment: health/database OK and both designated repository identities
with delivery_ready=true. It does not prove Runtime execution or A13 acceptance. For this operation use
the exact resume_condition:
`health=ok; database=ok; repositories=1360824360,1377749969; delivery_ready=true`.
Deployment/release operations require a host grant for their exact request,
revision and executable, registered by the operator when the concrete operation
is prepared. A grant does not permit bypassing required CI, changing budgets or
clearing product ownership. Missing grants remain visible as external waits.

Environment-hook failures and exhausted upstream transport retries have a
separate finite infrastructure budget. They preserve actual startup counts,
tokens and runtime. Unknown process exits remain charged code attempts. After
that budget is exhausted, explicit recovery can grant a single continuation
without resetting history or increasing global limits.

Preflight each stage's current prerequisites; do not require routes that this
Issue has yet to implement. Local checks use `tools/gate.py` and the same pinned
binaries/configuration as the formal gate. A local partial profile is diagnostic,
not evidence that the exact-head required Harness-Gate check passed. Keep the
implementation and its promised acceptance together; do not split out unmet
acceptance merely to obtain a merge.

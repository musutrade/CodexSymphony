---
tracker:
  kind: github
  provider:
    repo: musutrade/CodexSymphony
    token: $GITHUB_TOKEN
    symphony_lifecycle: true
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
  after_create: |
    set -eu
    timeout --kill-after=5s 280s git clone --depth 1 https://github.com/musutrade/CodexSymphony.git .
    git config --local core.hooksPath /dev/null
    git config --local credential.helper ''
    printf '\n.symphony-handoff.json\n.agent-cargo/\n.agent-tmp/\n' >> .git/info/exclude
  before_run: |
    set -eu
    test "$(codex --version)" = "$(cat codex-version.lock)"
    command -v cargo >/dev/null
    command -v node >/dev/null
    command -v npm >/dev/null
    python3 tools/gate.py config check
    mkdir -p target
    test -w target
agent:
  serial_delivery: true
  max_concurrent_agents: 1
  max_turns: 12
  max_retry_backoff_ms: 300000
  max_total_tokens: 100000000
  max_runtime_ms: 14400000
  max_consecutive_errors: 6
  max_run_attempts: 3
codex:
  command: >-
    /home/gem/.local/share/codexsymphony/symphony/codex-sandbox
    --config 'model_provider="openai"'
    --config 'model="gpt-6-astra"'
    --config 'model_reasoning_effort="high"'
    --config 'features.goals=false'
    --config 'tool_output_token_limit=2000'
    app-server
  approval_policy: never
  thread_sandbox: workspace-write
  turn_sandbox_policy:
    type: workspaceWrite
    networkAccess: true
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
use a worktree per Run, matching app-server cwd, protected Git metadata and
platform-owned Git delivery. Do not transplant this development controller's
development handoff protocol into the product. Cloudflare/Bark belong to 0b;
independent executor UID and the newly pinned Harness-Gate integration to 0c.

## Preparation and recovery

Before coding, check the Issue's declared tools, dependencies, writable build
paths and required test services in the actual execution sandbox. Use a
workspace-local target directory. Missing PostgreSQL or inaccessible network
is an environment blocker when required by this Issue, not a code defect.
The host hook is only a basic check; it does not establish sandbox readiness.

Use the injected `github_api` dynamic tool for GitHub operations. Symphony's
host adapter executes these requests with its configured credentials; the Agent
shell does not run git push, gh, or credential-bearing curl. Read-only local Git
commands remain useful. Protected .git is expected and is not a delivery blocker.
Do not expand writable roots or weaken network policy to publish code.
Before coding, confirm the tool is available and can read this repository and
Issue. Report missing tool/host authorization as a preparation blocker.
Do not read credential files or embed tokens in URLs, files, command output,
PR text or handoff summaries.

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
cargo test --workspace --locked, and cargo clippy --workspace --all-targets --locked
-- -D warnings, unless a reviewed repository policy defines a more precise scope.
Frontend changes use the checked-in package scripts and lockfile. Do not fabricate
commands, passing results, coverage, or evidence for absent code and infrastructure.
An applicable acceptance check that cannot run is incomplete, not N/A.

Do not run the real GitHub/cloud/notification spikes by default. Tests with
external side effects require task authorization and the designated test setup.
Keep production secrets out of this repository. Do not change CI, required checks,
this workflow or acceptance policy merely to make the current task pass.
For this repository run `python3 tools/gate.py config check` and the applicable
local checks before publishing the PR. The separately installed trusted host
runs `python3 tools/gate.py verify --profile ci --all` on the exact published
commit; GitHub Actions waits for its App-authenticated result. Agent workspaces
do not receive signing keys or reusable trusted runtime requests. CRAP <= 10 is
required; missing/unsupported measurements remain blockers. Do not delete quality
configuration, lower requiredness, substitute N/A, or reuse another source's
measurement evidence. Hook is partial assurance and cannot replace full CI.
Read .harness-gate/QUALITY.md and docs/remote-gate.md for provisioning and evidence.

Prefer focused reads and bounded output; preserve full evidence in artifacts.
Stop testing when the required checks pass unless new changes or failures justify
more testing. Do not use goal tools, spawn additional agents, or expand budgets.
The controller owns attempts, cumulative budgets and CI waiting.

## Delivery using the existing local controller

The controller creates symphony/GH-<number> from a fresh origin/main for new work.
Keep that branch and preserve dirty work on retries. Publish validated workspace
changes through `github_api`, executed by the Symphony host:

1. Read the expected remote branch/base commit and tree. Establish the intended
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
confirmed. Full host quality and CI acceptance remain pending until the
controller observes both required protected checks; do not claim them locally.
End the turn immediately afterward with "submitted; CI pending". Do not poll CI,
merge, close Issues or edit the host handoff journal. The controller handles
those actions. On an explicitly dispatched repair, reuse the same PR and branch,
validate the repair, publish through the host tool using the last confirmed
remote head as parent, replace the declaration and stop again.

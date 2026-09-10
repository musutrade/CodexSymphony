# CodexSymphony Architecture Boundaries

Status: **Normative architecture policy**

This document defines durable responsibility boundaries for CodexSymphony. Requirement lifecycle, execution lifecycle, external repository state, and validation evidence must not collapse into one state machine.

## 1. Three truth sources

CodexSymphony has exactly three state-bearing truth domains:

1. **Requirement = business truth**
   - What should be delivered.
   - Acceptance criteria, review readiness, business lifecycle, blocked/done decisions.
   - A Requirement is not completed merely because an AgentRun ended, a PR merged, or a validator returned PASS.

2. **AgentRun = execution fact**
   - What an agent actually attempted and produced.
   - Run lifecycle, worktree, tool calls, completion request, failure/interruption/budget state.
   - AgentRun records execution facts; it does not redefine Requirement business truth or GitHub state.

3. **GitHub / CI = external observation fact**
   - Commit, branch, PR, check-run, workflow, merge and repository observations owned by GitHub/CI.
   - CodexSymphony observes these facts and reconciles them; it must not invent them from local AgentRun state.

These truth domains may reference one another but must remain independently reconcilable.

## 2. Harness-Gate result is evidence, not a fourth truth source

**Harness-Gate Result = Validation Evidence.**

Harness-Gate may authoritatively decide whether a specific source/config/evidence identity satisfies its configured quality and validation policy, but that decision is evidence consumed by CodexSymphony orchestration. It is not a CodexSymphony lifecycle state source.

A Harness-Gate result should be treated as an immutable or identity-bound observation with fields conceptually equivalent to:

```text
validation_id
source_identity
config_identity
status
blockers
report/evidence/artifact references
producer/tool identity
observed_at
```

CodexSymphony must not model `HarnessGateStatus` as a peer lifecycle replacing Requirement, AgentRun, or GitHub/CI truth.

## 3. Forbidden state collapse

The following shortcuts are architecture violations:

```text
Harness-Gate PASS -> Requirement = done
Harness-Gate FAIL -> AgentRun = failed
AgentRun completed -> PR = merged
PR merged -> Requirement = done
local commit exists -> GitHub branch/PR exists
CI expected green -> GitHub check = success
```

Each transition must be justified by the domain that owns that fact and by orchestration policy.

Examples:

- A Harness-Gate FAIL can cause CodexSymphony to schedule a remediation AgentRun, mark a Requirement blocked, or request human intervention according to policy, but the validation result itself does not mutate those lifecycle facts directly.
- An AgentRun may complete successfully while producing code that fails validation.
- A PR may merge while a Requirement remains open because business acceptance or another required condition is still outstanding.
- A Requirement may be approved before any AgentRun exists.

## 4. Orchestrator responsibility

CodexSymphony owns **orchestration and reconciliation**, not validation semantics.

The orchestrator consumes:

```text
Requirement business truth
+ AgentRun execution facts
+ GitHub/CI external observations
+ Harness-Gate validation evidence
= next permitted action
```

Possible next actions include dispatching a new AgentRun, waiting for GitHub/CI, retrying an external action through the outbox, recording a blocker, requesting human intervention, merging when all independent preconditions are satisfied, or closing a Requirement according to its business acceptance policy.

The orchestrator must preserve provenance for every decision so a later reconciliation can explain which facts and evidence caused an action.

## 5. Validation boundary

CodexSymphony must not reimplement Harness-Gate quality semantics such as CRAP, coverage thresholds, baseline/ratchet/debt, collector trust, capability support, or aggregate quality decisions.

Likewise, Harness-Gate must not become the owner of CodexSymphony Requirement lifecycle, AgentRun lifecycle, retry budgets, scheduling, PR orchestration, or human-intervention state.

The integration boundary is therefore:

```text
CodexSymphony                         Harness-Gate
----------------                    ----------------
requirement lifecycle                validation policy
agent scheduling/run lifecycle       command/collector orchestration
GitHub reconciliation                evidence validation
retry/outbox/human intervention      quality/baseline/ratchet decision
next-action selection        <-----  identity-bound validation result
```

## 6. Project-owned validation

Application-specific tests remain owned by the application repository. CodexSymphony may request or observe Harness-Gate execution, but neither CodexSymphony nor Harness-Gate should reimplement project-specific API/E2E/integration/smoke/load/migration test frameworks.

Harness-Gate exposes generic command hooks, structured-result ingestion, and quality-collector boundaries. CodexSymphony consumes the resulting validation evidence and decides workflow actions.

## 7. Persistence guidance

Persistence should preserve separation rather than denormalize all states into one enum.

Recommended conceptual records:

```text
Requirement
AgentRun
GitHubObservation / CIObservation
ValidationEvidence
OrchestrationDecision / OutboxAction
```

`ValidationEvidence` references the relevant Requirement/Run/repository/commit where useful, but remains evidence with source/config identity. It is not authoritative for Requirement or AgentRun status.

## 8. Acceptance invariant

Any future implementation or schema change must be able to answer independently:

- What does the business currently require?
- What did the agent actually do?
- What does GitHub/CI actually report?
- What did Harness-Gate validate for this exact source/config identity?
- Which orchestration rule converted those independent facts into the next action?

If one stored status makes any of those questions impossible to answer independently, the design has collapsed truth domains and must be rejected or redesigned.

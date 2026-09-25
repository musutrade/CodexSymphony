# GH-121 preflight and continuation

Baseline: `034d803df21185305429681e2dac3a1e57d57af9`, branch `symphony/GH-121`.
Initial working tree was clean. No implementation source has been edited.
PR #123 (GH-118) and PR #127 (GH-120) were confirmed merged into main via authenticated GitHub API; see dependencies.json.
Local test PostgreSQL fixture is running with the pinned image and resource policy; real SQL `select 1` returned 1. Tool versions and Gate configuration diagnostic passed; see preflight.json.
`rg` is absent from the shell; fallback grep/find are available. An initial dbctl invocation omitted the required test/dev role; the corrected `status test` succeeded. These are not unresolved environment failures.

## Host prerequisite

GH-121 acceptance includes regression of the real GitHub delivery path. Local fixture connectivity and read-only repository API responses do not establish the deployed product health/database or its two designated repository delivery identities. The task explicitly provides the registered, read-only `product.identity_preflight` host operation for this deployment. Request that operation before continuing the integration work; do not read credentials or run real GitHub writes. Its receipt proves identity readiness only, not Runtime or A13 acceptance.

Resume after the host receipt satisfies exactly: `health=ok; database=ok; repositories=1360824360,1377749969; delivery_ready=true`.

## Implementation stage still outstanding

Read README, architecture boundaries, main specification chapters 23/24, controlled delivery plan, protocol and validation-Hook documentation. Only web/angular has a repository AGENTS.md discovered; no frontend edits planned yet.
P9 operation/registration/candidate types exist in controlled_contract.rs. validation_context::admit supplies generic candidate admission from GH-120. delivery_worker.rs currently operates on GitHub Pending/PR values and covers initial and replacement-head publication; merge paths also need review and migration. github_service.rs owns optional App configuration/client setup; preserve credential isolation and repository policy scope. project_hooks.rs supplies the four existing lifecycle Hooks. Do not treat the P9 types as implemented delivery adapters.

Next: inspect all delivery/merge/recovery entry points and implement the approved generic adapter boundary, credentials provider boundary, additional Hooks, durable reconciliation and migration without a second state machine. All GH-121 acceptance tests, full Rust suite, formatting, clippy, evidence finalization and complete local_gate remain required. No publication, real external write, deployment, CI success or task completion is claimed.

## Completed smoke checks

Runtime suite: 9 passed, exit 0 (terminal summary retained). Real Runtime transport/supervision: 1 passed, exit 0 (full log and exit file retained). Neither is the full workspace acceptance suite. Existing recovered handoffs concern GH-120 only; they are preserved unchanged.

## Continuation 2026-09-25: identity recovered, real-path execution unavailable

The host identity receipt now reports PASS for both designated repository IDs,
using independent delivery App 5069731. The earlier App permission failure is
historical and is not the current blocker. Health/database/identity readiness
is confirmed; no production failure is inferred.

Repeated local Runtime smoke: 9 passed, exit 0. Real Runtime transport: 1 passed,
exit 0. Retained logs: runtime-resume-smoke.log and runtime-real-resume-smoke.log.
Gate configuration diagnostic passed. No implementation source edits or publication.

Confirmed execution-surface limitation: github_api exposes GitHub REST, while
product.identity_preflight is read-only. Neither can invoke the Rust product
adapter with its deployment-owned credential provider. The prepared environment
has no GITHUB_APP_CONFIG/RUNTIME_CONFIG and provides ordinary local fixture/test
commands, not an authorized real-product delivery runner. See
real-delivery-capability-preflight.json for non-secret inventory and file hashes.
No private key/token was read, copied or requested. Whether a suitable host runner
exists outside the exposed environment is unknown; this report does not claim
that the host lacks one.

Next operator action: expose the authorized bounded real-delivery test setup through
the credential-owning host, including the executable operation, designated scenario,
repository scope and frozen validation policy. It must exercise the Rust adapter
and return source/candidate/operation/remote identity evidence; direct development
GitHub writes and the historical echo-only workflow are not substitutes. No new
production deployment or write authorization is inferred from identity readiness.

Resume at acceptance setup preflight, then implement GH-121; all implementation,
controlled regressions, real GitHub regression, full workspace checks and complete
exact-tree local_gate remain outstanding. Existing budgets, frozen tasks, evidence
and baseline remain unchanged. No rollback is needed because source was not edited.

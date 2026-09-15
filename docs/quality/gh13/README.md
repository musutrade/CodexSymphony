# GH-13 implementation and local validation

Status: issue-local acceptance passed on baseline `9ca3d94503584aca9f13c11ed8f8dec535347772`.
Publication and the exact-head trusted host checks are separate from these local
results. This is not product-wide 0a acceptance.

## Delivered behavior in this workspace

- First Project/Repository configuration, one immutable remote identity, Draft
  creation/list/detail/edit, Ready review and withdrawal, SQL migration and events.
- Pure Contract/policy rules in `apps/server/src/contract.rs`. ACs reference named
  validation steps; steps select `cargo_test` or `npm_test` and a safe test selector,
  expected result and timeout. Future tests need not exist yet. No executor or
  host shell command is implemented by these APIs.
- Ready atomically stores a new revision, ordered generated AC IDs, full Contract,
  repository identity, policy/budget version and local reviewer. Creation/review
  times are persisted in the requirement/event records. Snapshot AC IDs correspond
  to the same-index acceptance criterion in the frozen Contract.
- All writes use the existing Host/Origin/CSRF middleware. An object version and
  request ID protect each mutation; Ready also checks the policy version actually
  shown for review. Reusing an ID with different input conflicts; identical retries
  return the original persisted result without another event or snapshot.
- A single transaction advisory lock serializes these low-volume control-plane
  mutations, including policy changes/revocation. It does not claim Run ownership.
- Policy changes do not rewrite snapshots. Revocation advances a permanent version
  cutoff; restoring the repository does not revive older Ready authorizations.
  `authorization_valid` on a fresh detail read reports that distinction. Historical
  idempotency results remain historical; downstream actions must recheck live
  authorization and capabilities transactionally when they are implemented.
- `/requirements` provides repository setup, form/list, explicit saved-version
  review and withdrawal. Local user identity is intentionally not M2 authentication.
  Runtime and repository delivery readiness stay false; deployment network is
  explicitly unconfigured/unverified. Network intent grants no task-specific rights.
  Closing the browser has no queue deletion or execution lifecycle effect.

Repository configuration is exposed by API; this issue's UI only registers the
initial repository. Configuration updates must supply a reason and current version.
There is no budget-increase or code-repair override on a Requirement. No downstream
scheduler, actual network enforcement, GitHub delivery or login is claimed.

## Commands and results (2026-09-15 continuation)

GH-12 PR #31 is merged; both required checks on its head
`254eb250bdca51011422caa2b9f044aa628b8142` passed. The assigned branch retains its
implementation on current main, including reviewed collector recovery PR #33.
No quality thresholds, required checks, signing inputs or controller files changed.

Database commands use the provisioned launcher and disposable fixture. Build
outputs remain under workspace `target/`. Full raw logs and captures are retained
locally; `local-validation.json` records summaries and their hashes for review.

| Actual command | Result / retained local evidence |
|---|---|
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/verify.py` | PASS: test/dev SELECT 1, test discard, dev retention and read-only boundaries; `target/gh13-environment-verified.json` |
| `npm ci --offline --no-audit --no-fund` in `web/angular` | PASS; `target/gh13-npm-ci-resume.log` |
| `cargo fmt --all -- --check` | PASS |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | PASS: 12 tests; `target/gh13-rust-tests-resume.log` |
| `python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; `target/gh13-clippy-resume.log` |
| `npm run lint`, `npm test -- --watch=false`, `npm run build` in `web/angular` | PASS: 11 unit tests; `target/gh13-ui-{lint,tests,build}-final.log` |
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py` | PASS: real API build/start, 10 browser tests; `target/gh13-e2e-final.log` |
| `python3 tools/gate.py config check` | PASS; `target/gh13-gate-config-resume.log` |
| Rust rc.2 capture and independent raw re-export (below) | PASS: 68 source functions, minimum line coverage 90%, minimum region coverage 81.08%, maximum CRAP 10; `target/gh13-rust-risk-final/` |
| `HARNESS_GATE_TYPESCRIPT_PLUGIN=/home/gem/.local/share/harness-gate/typescript/0.1.0-rc.4/node_modules/@harness-gate/typescript-collector node web/angular/tools/probe-typescript-risk.cjs` | PASS: runtime file line coverage 100%, 32 functions, maximum CRAP 6; `target/gh13-ts-risk-final.log` records raw artifact directory |
| `python3 /opt/symphony-env/run.py python3 tools/check-requirement-contract.py` (after disposable fixture cleanup) | 19 live variants: 18 business plus health 200; `target/gh13-contract-observations.json` |
| `node target/gh13-measure-business.cjs` | rc.4 schema/type rehearsal: compatible, no client drift; `target/gh13-business-contract-metrics.json` |

Rust capture command:

```sh
python3 /opt/symphony-env/run.py python3 \
  /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.2/capture.py \
  --repository . --output target/gh13-rust-risk-final \
  --target-dir target/gh13-cov --source-root apps/server/src \
  --input Cargo.toml --input Cargo.lock --input apps --input migrations \
  --manifest apps/server/Cargo.toml
```

The installed plugin's `reexport` re-read retained native objects, profiles and
bound source to produce `measurements.json`; no historical measurements were
substituted. The resulting backend series matches the reviewed configuration.
These unsigned rehearsals do not replace trusted host authorization or CI.

## Acceptance and limits

API tests cover normal/invalid Contracts, future test selectors,
stale PATCH, competing reviewers, repeated Ready/withdrawal, immutable snapshots
after policy updates, revocation without revival, Running/Submitted edit rejection,
and Host/Origin/CSRF denial without requirement/event changes. Invalid request IDs
are rejected. Database unavailability returns a bounded 503; corrupt stored input
cannot produce a Ready revision, event or idempotency result.

Desktop Chrome and Pixel 7 complete create → edit → review Ready → reload →
withdraw against the real API. Tests check loading, empty, failure and submission
states, form-error associations, keyboard-visible focus and Enter submission,
no horizontal overflow, and axe WCAG A/AA. Full-page review and withdrawal
screenshots remain in `web/angular/test-results/`; desktop/mobile review images
were visually inspected. Common styles use the committed arc-admin tokens;
Material focus contrast was corrected from the earlier observed 4.37 ratio.

The HTTP rehearsal checks full generated types and all declared business variants.
It compares the business spec with itself, so it is **not** a historical baseline
compatibility measurement. Existing real unavailable-pool API tests cover health
503. Full live health 503, trusted baseline comparison, signatures and exact-head
CI remain the installed host's responsibility.

No scheduler, Run execution, GitHub delivery, static network enforcement or M2
login is claimed. Ready remains durable and visibly awaiting downstream readiness.
This closes only A01 input and A12 form portions of the wider 0a acceptance map.

## Recovery history

Previous local records under `target/gh13-rust-collector-blocker/` document rc.1
rejecting Serde metadata and expression macros. Reviewed PR #33 installed rc.2;
a fresh capture now maps all production callables. It exposed real issue-code
coverage gaps and CRAP 10.009 in the transaction entry point. This continuation
added storage-failure/rollback tests, moved request-ID validation into the pure
domain module and simplified row decoding. Fresh measurements above meet the
unchanged limits. The earlier block is resolved by this evidence, not by a waiver.

Final frontend measurement also checks the application shell: navigation is bound
from component configuration, with tested current-page ARIA semantics. All runtime
files have measured line coverage. Only the generated declaration-only type file
belongs to the existing declaration group; no runtime file was exempted.

> 已恢复：此文保留历史失败。当前接口与验证见 [http-fixture-recovery.md](http-fixture-recovery.md)。

# GH-22 blocked development checkpoint — 2026-09-16

This is incomplete implementation, not a PR delivery or acceptance claim.
Branch: `symphony/GH-22`; baseline: `90994dec3cd3e355e6d792ebd94f4594b0d9634e`.
GH-21 PR #48 was merged, and both checks on its published head passed.

## Confirmed capability gap

The checked-in trusted HTTP capture interface (`tools/quality-host/capture.py`
and `http_scenarios.py`) starts the API without `RUNTIME_CONFIG` and accepts only
HTTP scenario steps. It has no Runtime fixture preparation input. The runtime
service explicitly remains disabled without this configuration. A clean database
and the existing public requirement/control operations cannot produce a business
question or owned runtime evidence.

The installed HTTP collector rc.4 requires every declared operation/status variant.
New production Angular consumers include successful persisted-question answering
and owned evidence reading. These need genuine provider observations. Removing
these success variants, accepting fake responses, or adding a production fixture
creation endpoint is not an acceptable workaround. Updating the independent
verifier's preparation capability requires operator provisioning; this checkout
cannot install changes into that service.

This identifies the supported checked-in host interface, not a claim to have
inspected the running verifier's private deployment or credentials. If the
verifier already has a newer approved fixture mechanism, supply its documented
interface and demonstrate the cases below.

## Reproduction and retained evidence

All raw evidence is in `target/gh22-blocker/` in this preserved workspace.

- `contract-capability-repro.py`, run with the supplied environment wrapper:
  actual operations and inbox GETs return 200; a question-answer POST and owned
  evidence GET without Runtime fixtures return 404. The scenario parser rejects a
  fixture setup field with `unknown capture scenario field`.
- `contract-capability.json`: actual status/body results; no fabricated success.
- `http-observations.json`: 25 real existing contract observations collected after
  recreating only the disposable test fixture.
- `collector-reproduction.log`: actual installed collector rejects incomplete
  collection with `missing provider response variant`.
- `source-identities.json`: SHA-256 identities of the relevant sources and contract.

The collector also lists health 503 and new ordinary read/control observations
that are absent from this local rehearsal. Health 503 remains the normal host
responsibility; the ordinary new scenarios still need implementation. Those
entries are not the external blocker. The blocker is preparation of the genuine
Runtime-dependent successful response variants.

## Preserved work and actual checks

Uncommitted changes include versioned/idempotent operator controls; shared
transaction helpers for existing stop/resume/cancel behavior; independent read
models and owned database evidence previews; initial metrics migration; initial
four-page navigation, detail/inbox/list views and generated HTTP types; and Rust
business tests. Preserve all tracked and untracked source changes on retry.

Passed before or during this checkpoint:

- `python3 tools/gate.py config check`.
- `npm ci --offline` in `web/angular`.
- `python3 /opt/symphony-env/run.py cargo test --workspace --locked --test health --test runtime_real`: 6 health tests and 1 real Runtime test.
- Existing browser baseline via `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py`: 10 Desktop Chrome / Pixel 7 tests.
- One full `cargo test --workspace --locked` run through the wrapper passed before the latest operator additions.
- `python3 /opt/symphony-env/run.py cargo test --workspace --locked --test operators`: 2 new tests, including CAS/replay, pause/answer, duplicate/stale answers, cancellation state, evidence ownership and redaction.
- Initial new detail/inbox Angular build passed before subsequent list/metric edits.

These checks cover different saved source checkpoints and do not establish final
acceptance of the current tree. `frontend-lint.log` records generated-type style
errors still needing normal formatter/ESLint fixes. New browser business tests,
unit coverage, final build/lint, final full Rust checks and real CRAP/coverage
measurements remain incomplete. No signing or CI success is claimed.

## Resume

Operator next action: provision/document a reviewed fixture preparation mechanism
for the independent verifier that creates real persisted Runtime questions and
owned evidence without GitHub writes, production test APIs, or developer signing
keys. Retain current thresholds and independent exact-commit verification.

Machine-checkable recovery condition: the approved capture run obtains HTTP 200
from both `POST /api/operator/questions/{id}/answer` and
`GET /api/requirements/{id}/evidence/{run}/{channel}` for fixture-owned identities,
and the installed collector accepts those actual request/response observations
against the checked-in schema. The setup must be repeatable from a clean database.

Resume at implementation/integration, not publication: complete action
availability/version integration, measurements, contract scenarios and errors,
frontend state/accessibility tests, review security/metrics details, and finish
all required local checks before creating a focused PR. Full evidence lifecycle
persistence remains the explicitly downstream GH-23 integration.

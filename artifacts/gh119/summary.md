# GH-119 validation evidence

Base: `549a8c137f7fb982306aa174195ea9e9cc3384bb` (merged prerequisite PR #123).
Branch: `symphony/GH-119`. Source hashes: `source.json`.
Earlier environment fingerprint: `bc185798f498bb3ff912c3c1dbc522ad77c88c6f6b4e323fd2c2fa1fce2a5e27`;
full tool/resource fingerprint is emitted by the prepared test wrapper in the Rust logs.
The disposable test fixture and synthetic persistent fixture both returned SQL `1`
before implementation. Only the disposable fixture was recreated by the supplied wrapper.

## Acceptance mapping

1. Repository environment plan reuses the controlled contract and references existing lockfiles.
   Repository/version/scope, host configuration, executable bytes and approved plan are bound;
   no installation/upgrade endpoint is exposed. Frozen review snapshots drive task admission.
2. Two separate local Git repositories run real Python and Node tools/tests in separate resources.
   Neither project requires a database; Python disables cache, Node exercises optional cache.
   PostgreSQL in these tests stores platform facts, not project data.
3. Enable, platform startup, preparation, launch, validation, delivery and recovery invoke applicable
   admission. CLI applies the same test role for configured CI; absent CI returns not_applicable.
   Declared tool/image/memory/scheduling changes produce expected/actual differences.
4. Real Unix socket service retains its loaded config identity. New disk configuration with an old
   effective config and a replaced installed binary with an old running executable are rejected.
   Extension observations include syntax/semantic validation and config digests.
5. Explicit dev/test roles and approved mutable runtime alternatives; missing/forged/failed evidence,
   identity changes and unsupported role changes fail closed. No CI probe is required when disabled.
6. Optional cache scope, identity, permissions, capacity, seed manifest/content and symlink checks.
   Disabled cache creates no cache resource. Seed writes remain an operator operation.
7. Environment monotonic elapsed time plus preparation, validation, delivery and CI execution records
   supply available timing. Unavailable subphases/hits are unknown; disabled cache is not_applicable.
   Real project test commands remain required; cache does not supply validation success.
8. Timeout stops descendants; unresolved intent blocks subsequent probes. Original preparation
   deadline, authorization, review revision and budget survive environment failure. No model repair
   or model upgrade is requested. Migration/rollback/cleanup: docs/repository-environments.md.

## Commands and results

- Baseline: prepared full Rust suite PASS (`baseline-tests.log.gz`).
- `cargo fmt --all -- --check`: PASS (`fmt-quality.log`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: PASS (`clippy-quality.log`).
- Final `python3 /opt/symphony-env/workspace_tests.py`: PASS, exit 0 (`workspace-quality.log`).
  This executes the required locked workspace suite with serial scheduling, including real Runtime.
- `python3 -m unittest discover -s tools/symphony -p test_trusted_preparation.py`: 3 PASS.
- Offline locked `npm ci`; `npm run lint`, `npm test`, `npm run build`: PASS.
  Frontend: 13 test files / 47 tests. Build retains the existing bundle-size warning (exit 0).
- Supplied `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py`: startup failed because its HTTP WEB_ORIGIN conflicts with the existing HTTPS requirement (frontend-e2e.log and frontend-e2e-startup.log).
- Existing supported `python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py`: PASS, exit 0; 28 desktop/mobile tests (frontend-https-e2e.log.gz); final backend refactor also PASS (28 tests, exit 0) in frontend-https-refactor.log.gz. No host script or HTTPS policy changed.
- `python3 tools/gate.py config check`: PASS (diagnostic only).
- Complete installed local Gate is requested after finalizing this evidence manifest. Its exact-tree
  host receipt is external to this immutable source; delivery is conditional on its PASS.

Initial complete Gate failed because its pinned source collector rejected custom Serde field attributes; explicit serialization preserves the same wire representation and old identities. A direct inventory diagnostic then found complexity from fallible operations, so admission, evidence, cache traversal and serialization responsibilities were split without changing policy. The unsupported metadata diagnostic now has no failures. Two in-progress regressions were intentionally interrupted (exit 130) for these further edits; workspace-complete.log.gz is the final required run. The next complete Gate rejected an exact LLVM anchor for a matches! closure in lockfile path validation (local-gate-anchor-failure.json). The predicate was extracted into a normal function with unchanged behavior; workspace-anchor.log.gz is the subsequent final required regression. Full Gate must be rerun on the new tree.

Initial Clippy and backward-compatible snapshot tests failed during development; fixes and subsequent
passing logs are retained. Earlier intermediate logs do not substitute for the final run.

## Limits

Image/memory/scheduling observations are controlled fixture simulations. Tool execution, local Git,
project test commands, actual service process/socket, supervisor termination and platform DB are real.
No production deployment, real external GitHub/cloud spike or production container probe was run.
The trusted phase's POSIX seed checks do not claim adversarial same-UID isolation; strong deployment
write boundaries require operator-owned read-only mounts. Cache capacity is checked at admission;
ongoing hard quotas belong to the reviewed host storage adapter.
Complete two-repository product workflows and later delivery extensions remain their declared Issues.
Local acceptance does not assert the two protected remote checks; those remain controller-owned.
The host archive receipt is not accessible to the Agent and is not claimed here.

The third complete Gate identified an error-conversion closure anchor in coordinator.rs.
A complete local llvm-cov run then passed all tests and produced exact-source diagnostics:
one deadline-access closure still lacked a native callable mapping; the available mappings
also identified Profile validation and preparation admission risk above 10. This diagnostic
is explicitly incomplete, never an acceptance substitute (mapping-diagnostic.json.gz).
The error conversion uses an explicit intermediate value; the deadline now uses direct field
access without a closure. Host configuration rejection tests cover unsafe paths, credentials,
and timeout; preparation environment-failure recording is a separate responsibility.
The required final ordinary fixture run is workspace-mapping.log.gz; complete Gate must confirm
the resulting source's coverage, mappings and risk. Earlier raw diagnostics apply only to their
recorded earlier source; no missing mapping or measurement is treated as PASS.

The fourth complete Gate reached quality evaluation and failed per-function coverage plus a
marginal existing Hook classification risk. Tests now exercise real authenticated configuration
rejection, preparation blocking/recovery with cumulative attempts, nested seeds, evidence paths,
oversized input, timestamp boundaries, inconsistent evaluation and interrupted serialization.
Repeated protocol error conversion shares one formatter; Hook error classification retains its
behavior with deterministic stopped/unknown/timeout/cancelled receipt tests. All affected measured
functions meet 80% line/region and CRAP <=10 in the final targeted diagnostic including Runtime
(admission-mapping.json). This is diagnostic scope, not complete Gate acceptance. The final required
ordinary full fixture suite is workspace-quality.log. No policy or requiredness was changed.

## Resumed validation (2026-09-24)

The retained workspace includes the provisioned Rust source collector rc.7 package,
measurement identity and environment lock projection. The current prepared wrapper
accepted fingerprint `64b2dec364e628450605ed2ad950295de37bcf5b29f226c729e674b1e8f39529`.
No collector threshold or requiredness was lowered. PR #123 was read back as merged
at the recorded base. The interrupted workspace-quality.log is historical, not PASS.

The first resumed full suite exited 101: environment fixture deadlines (5 seconds
and 1 second) expired amid durable writes taking roughly 1–3 seconds each. The
retained report/file timestamps are in resume-timeout-diagnostic.json. Test fixtures
now allow 60 seconds for normal probes and 20 seconds for the intentional timeout,
whose child sleeps 90 seconds. The timeout/quiescence assertions remain required;
production deadline behavior is unchanged. workspace-resume.log retains the failure.

Final ordinary acceptance: prepared full `cargo test --workspace --locked` via
`python3 /opt/symphony-env/workspace_tests.py` PASS, exit 0
(workspace-resume-final.log), including real Runtime and all 8 environment tests.
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 -m unittest tools.symphony.test_trusted_preparation` (3 tests), and
`python3 tools/gate.py config check` PASS, exit 0 (respective resume logs).
The complete local Gate is the remaining publication prerequisite; its exact-tree
receipt remains host-owned. Remote branch/main reconciliation is recorded in
remote-reconciliation.json. Evidence manifest: `.symphony-evidence.json`.

The resumed complete Gate rejected exact source mappings for the two newly added
inline cfg(test) modules, including their test closure (local-gate-resume-failure.json).
They now follow the repository's existing path-based unit-test layout under tests/unit;
assertions and production behavior are unchanged, and no production function was removed
from measurement. The final full prepared suite is workspace-unit-layout.log (PASS,
exit 0), with fmt-unit-layout.log and clippy-unit-layout.log also PASS. A new complete
Gate run must validate this final source; previous diagnostics are not a PASS substitute.

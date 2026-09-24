# GH-118 validation and delivery scope

Baseline: `761f09a7c2021b1a157ca9d6a1d4309068e1b54a`, branch `symphony/GH-118`.
PR #111 and #113 are merged into main; read-back identities are in prerequisites.json.
Actual namespace/environment fingerprint matches the provisioned fingerprint, with
writable target/tmp and SELECT 1 against both supplied fixtures (preflight.json).
The full-suite entry recreates only the disposable fixture and records its resource
policy and final status. No delivery credentials or Gate signing material were used.

The two supplied proposal originals are preserved after an explicit current-authorization
notice. document-checks.log verifies original SHA-256 and local link targets.
The plan original is `46e27cc5b4a2d73106a948e1bc99b118e9fd5ff219d1d88e3e5446f7961e3fb7`.

## Acceptance mapping

| Issue criterion | Implementation / evidence |
|---|---|
| 1 Core/extension responsibilities | extension-protocol P1/P9, architecture boundaries section 9; core config contains capability/scope/config/provider references |
| 2 Operations and lifecycle | P3 preserves four HookEvents; P9.3 defines environment, validation, delivery and post-delivery inputs/results |
| 3 Identity, completeness, invalidation | controlled_contract Call/Evaluation and P9.2; exact identity/expiry/missing/duplicate/unknown tests |
| 4 Credential and result trust | P2/P9.2/P9.3 separate credentialed implementation from candidate execution; structural checks explicitly are not source authentication |
| 5 Provider-independent facts | P6/P9.3 separate Submitted/Done and phases from GitHub facts; local_git creates no PR/CI facts |
| 6 Reuse and reconciliation | P9.3/P10 map existing coordinator, validation, delivery and recovery records; unknown effects reconcile before retry |
| 7 Compatibility and source coordination | Proposal originals, requirements/main spec/README updated; existing extension_contract tests preserve legacy mapping; six new controlled_contract tests reject unknown versions/capabilities and permit a non-Rust/no-GitHub/no-cache environment |

Raw test output is stored losslessly in gzip files; `gzip -dc` reproduces it.

## Checks

- `python3 tools/gate.py config check`: PASS (gate-config.log).
- `cargo fmt --all -- --check`: PASS (fmt.log).
- Pre-refactor `python3 /opt/symphony-env/run.py cargo test --locked --test controlled_contract`:
  PASS, six tests (contract-tests.log.gz).
- Pre-refactor `cargo clippy --workspace --all-targets --locked -- -D warnings`: PASS (clippy.log).
  clippy-initial.log retains the earlier failed run that overlapped an interface edit;
  the final rerun used the settled interface and exited 0.
- Initial `python3 /opt/symphony-env/workspace_tests.py`: PASS (baseline-tests.log.gz),
  including real pinned Codex Runtime. Started before the new test target existed;
  this is not substituted for the final suite.
- Pre-refactor `python3 /opt/symphony-env/workspace_tests.py`: PASS, 278 passed, 0 failed, 0 ignored (tests.log.gz), exit 0.
- Final post-refactor `python3 /opt/symphony-env/workspace_tests.py`: PASS,
  summed test-result counts 278 passed, 0 failed, 0 ignored, exit 0
  (tests-refactor.log.gz), including all six contract tests and real Runtime.
- Final post-refactor `cargo clippy --workspace --all-targets --locked -- -D warnings`:
  PASS, exit 0 (clippy-refactor.log); format check also rerun after the refactor.
- Initial complete `local_gate` stopped at frontend preflight: `ng: not found`
  (local-gate-initial.json). `npm ci --offline --cache ../../.agent-env/npm-cache`
  in web/angular then completed with exit 0 (npm-ci.log); no lockfile or policy changed.
- The next complete Gate passed all execution steps but rejected
  Evaluation::check_pass with CRAP 12 > 10 (local-gate-quality-failure.json).
  Identity/expiry, completeness and per-check verification were separated into
  focused functions; duplicate verification was removed. No policy changed.
- The following host Gate attempt exited 101 during `cargo llvm-cov` capture,
  without returning its nested Cargo stdout/stderr. Those host paths are not visible
  here (local-gate-capture-failure.json, host-log-access.txt); the cause is unconfirmed.
  A same-command diagnostic in the supplied environment, with a recreated disposable
  fixture, pinned settings and fresh short TMPDIR, passed all tests and LLVM export:
  `cargo llvm-cov --manifest-path apps/server/Cargo.toml --locked --json
  --output-path .agent-tmp/gh118-coverage.json --verbose`, exit 0
  (coverage-diagnostic.log.gz). This is not the isolated host namespace/source copy
  and is not a signed quality measurement. No source change followed that failure.
  A fresh complete Gate is required; the local diagnostic does not override it.
- Complete independent prepublication `local_gate`: required after evidence/source
  freeze; its exact-tree receipt and result are recorded by the host and reported in
  the handoff. These ordinary local logs do not claim signed Gate or remote CI success.

## Boundaries and rollback

This issue provides the unified contract and minimal pure compatibility checks.
It does not dispatch new operations, add a database migration, install extensions,
change credentials, deploy a service, or prove a real local_git product lifecycle.
Actual provenance and artifact-byte verification remain mandatory at the future
supervised call sites; matching a returned Call alone is not authentication.
Environment integration, complete validation, delivery adapters and real two-mode
acceptance remain #119/#120/#121/#105/#122, respectively. No real GitHub/cloud spikes
were run. Four lifecycle hooks and existing Runtime/validation/delivery behavior
remain covered by the existing suite. No policy thresholds or required checks changed.

Existing frozen config hashes, records, budgets and evidence are retained. The new
optional companion config has its own approved digest. No persisted data migration
is needed for GH-118 rollback. Later adapters must pause new claims, quiesce/preserve,
reconcile unknown effects and use a version compatible with in-flight records before
rolling back. The Elixir development handoff is not a Rust product capability.

Evidence manifest: `.symphony-evidence.json`; source files/digests: source.json.
The host after_run archive is outside this workspace; no archive completion is claimed.

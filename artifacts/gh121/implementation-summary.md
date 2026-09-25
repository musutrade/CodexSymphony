# GH-121 implementation and continuation

Baseline: `034d803df21185305429681e2dac3a1e57d57af9`, branch `symphony/GH-121`.
Prerequisite PRs #123 and #127 were reconfirmed merged into main. Existing dirty
recovery/evidence files were preserved. No production deployment or GitHub write
was performed by this continuation. The runner setup request
`fd898aa748644afe885784f1eb0d1db1` returned ok; its baseline PASS is not new-candidate
acceptance.

## Saved implementation

- `delivery_extension`: vendor-independent capability/submit/observe/reconcile
  dispatch, admission before and after auxiliary hooks, intent-before-send,
  receipt retention before identity/verification and applicable post validation.
- Native publication/merge adapters reuse the existing outbox and merge ledger.
  Initial, repaired, updated and cancellation writes use the boundary. Native
  GitHub code rechecks head and applicable base/capability after hooks.
- `delivery_hooks` / `delivery_hook_process`: deployment allowlist, exact plan and
  stage digests, scoped P9 inputs, original Hook invocation ledger, bounded
  credential-free subreaper execution, raw evidence and stop proof. Lost outcomes
  are reconciled without replay; cancellation and cleanup retain unresolved Hooks.
- `github_credentials`: deployment-owned key loading and delivery/check identity
  separation. Local-only configurations do not load/probe an App. Actual local_git
  execution remains assigned to #105; no local delivery success is fabricated.
- Migration 0034 adds nullable input to the existing invocation ledger. Empty
  host extension lists retain previous profile digests. Migration/rollback and
  deployment details are in `docs/delivery-extensions.md`.

## Evidence so far

- Preflight: current-runtime-smoke.log (9 Runtime + 1 real transport test),
  current-preflight.json, gate-config.log and preserved provisioning reports.
- workspace-tests.log is the retained first failure: an old controlled fixture
  reused the delivery App as trusted Checks publisher. Fixture identities were
  separated; the isolation check was kept.
- workspace-tests-current.log/.exit: full suite passed, 39 suites / 302 tests,
  exit 0, before the final additional fault/recovery changes.
- hook-tests-final.log/.exit: final auxiliary Hook scenarios passed, including
  repeated calls, result loss, implementation/stage tampering, changed inputs,
  known failure, timeout, output overflow and candidate mutation.
- implementation-inventory.json records 704 source/config/test/doc files and
  modes. final-test-binding.json binds the final full suite to its SHA-256.
- Final full-suite output: workspace-tests-final.log/.exit: 39 suites, 304 tests passed, exit 0. Source and executable modes match all 704 inventory entries.
- clippy-final.log/.exit: all targets with `-D warnings`, exit 0.
- fmt.log/.exit and gate-config.log: passed diagnostics.
- validation-summary.json binds the commands, outcomes, source inventory and environment fingerprint; complete Gate and new-candidate real regression remain pending.

## Remaining authorized stages

After final local validation succeeds, freeze a local source commit/tree and
request `product.real_delivery_candidate_install` exactly as the supplied
REAL-DELIVERY-RUNNER.md requires. The host must rebuild/review the driver against
this source, retain credential isolation and the designated repository/branch
scope, and expose matching setup evidence before the new-candidate real test.
Do not substitute the existing baseline PR or development github_api writes.

Then complete the candidate-bound real regression, finalize the evidence manifest,
run the required complete local_gate, fix any actual quality failures, and publish
only a current exact-tree PASS through the host Git Data API. No Gate PASS,
CRAP/coverage measurement, PR delivery, remote CI or product-wide acceptance has
been claimed. Local source/evidence may still require a new Gate after host
regression receipts arrive. Do not reuse a receipt from an earlier tree.

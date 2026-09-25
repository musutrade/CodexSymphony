# GH-120 quality recovery

The complete host export for run-59d426e355ed resolved the earlier truncated-diagnostics blocker. all-failures.json records eleven failures across ten subjects: one unexecuted error conversion closure and nine CRAP failures. Thresholds and required measurements are unchanged.

Validation is now split by responsibilities: frozen registration and Call construction, retained payload decoding and policy admission, canonical path checking, invocation/evaluation retention, accepted-record reconciliation, unknown-outcome recording, supervisor settlement, startup checks and heartbeat. Admission conditions and persisted identities are preserved. The error conversion uses eager ok_or instead of a lazy constant closure. A deterministic startup-deadline test verifies a missing process identity creates the original stop request and does not relaunch; an existing identity avoids the startup timeout.

The first local full-suite compilation in this recovery found the test used an unavailable uuid dependency. The fixture now uses the test process ID and removes its directory on completion. The failed attempt remains in workspace-tests-quality-recovery.log; current results are exclusively quality-fixed-checks.json. No dependency or quality policy was changed.

source-inventory-quality-fixed.json identifies current source bytes and modes. quality-preflight.log verifies tools, database connectivity and writable build paths. The full prepared suite captures the canonical environment fingerprint and fixture policy. The prior normal-path-verified evidence remains historical; normal-path-quality-fixed is the new real Git/process/PostgreSQL fixture capture. Real Runtime integration is included in the full suite; no real GitHub/cloud/notification side effects are exercised.

Migration and compatible rollback remain in docs/validation-hooks.md. The exact-tree complete local_gate must pass after the final evidence manifest is written. Neither ordinary checks nor this pre-Gate summary claims Gate PASS or protected CI acceptance. The handoff will record the host-returned publication SHA only after readback. No host archive copy is claimed.

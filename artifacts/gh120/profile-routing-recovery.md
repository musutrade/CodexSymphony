# GH-120 coverage subprocess recovery

The host-exported diagnostics for run-64a11aabddc5 identify an instrumented validation supervisor writing default_1580041910218139655_0_3841.profraw inside the candidate checkout. Compilation succeeded; candidate cleanliness correctly rejected delivery. The prior missing-diagnostics blocker is resolved by the retained host-diagnostics-run-64a11aabddc5/ evidence, not by inference.

process.rs now preserves the verifier-provided LLVM_PROFILE_FILE through both hook and development environment allowlists. Coverage output remains at the verifier-selected destination; the candidate cleanliness check, required quality measurements, policy, and credential exclusions remain in force. The complete installed Gate must still prove the instrumented path succeeds.

The targeted P9 environment integration passed. Current required check commands, exit codes and logs are in profile-routing-checks.json. source-inventory-profile-routing.json binds source and modes. Earlier summaries and normal-path evidence remain historical and are not relabeled as current-source Gate results. Migration and rollback remain documented in docs/validation-hooks.md.

Preflight confirmed the test fixture running with its approved PostgreSQL image/resources, SELECT 1 over the injected test URL, writable target and /tmp, and installed Cargo/Rust/Python/Node/npm. rg is absent, so bounded standard searches were used. The initial dbctl invocation lacked its role argument and a plain psql invocation did not consume TEST_DATABASE_URL; corrected documented fixture/explicit URL calls succeeded without environment changes. PRs 123 and 125 are merged into main, which remains 93282b2d64d3bed1a7cfc1c7e8aecefc3935187b; the GH-120 remote branch is absent.

No external product write, deployment, merge, issue closure or CI polling has occurred. Publication requires complete local_gate PASS for the finalized tree. The handoff will identify the actual published SHA; protected CI remains independent.

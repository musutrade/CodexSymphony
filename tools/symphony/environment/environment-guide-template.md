# Trusted project development environment

This environment runs trusted owner-controlled code using Symphony's execution
model. Agent command execution uses danger-full-access inside the prepared
project environment. No command-level sandbox or managed-network allowlist is
required. Local Git is writable. Host GitHub credentials and independent Gate
signing keys are not mounted here.

Use normal project commands. `python3 /opt/symphony-env/run.py COMMAND ...` only
injects TEST_DATABASE_URL, DEV_DATABASE_URL and DATABASE_URL for the supplied
PostgreSQL fixtures and executes COMMAND directly. `dbctl.py status|recreate
test|dev` manages these fixed fixtures. The test fixture is disposable; the dev
fixture contains synthetic data and persists across recreation.

Before coding: check dependencies, writable target/tmp paths, actual database
connectivity and the relevant project smoke tests. Run `cargo test --workspace
--locked`, including runtime_real, directly through the fixture settings wrapper.
No reviewed-runtime manifest, copied binary, runtime_product_acceptance.py or
backend_tests.py host receipt exists in this model. Old reports requesting those
entries describe the superseded environment; preserve their history, do not
recreate their requirements.

Use the read-only arc-admin reference and supplied browser/npm caches for UI
work. `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py` starts the
API and runs the normal frontend tests. Network uses the development environment's
normal connectivity; project network declarations describe dependencies, not
per-task isolation.

GitHub operations use the injected authorized github_api tool; no GitHub secrets
are provided to the shell. The final exact-commit Harness-Gate and Trusted
Harness-Gate run in the independent verification service. Local tests do not
replace these checks or modify their requiredness, thresholds or signing policy.

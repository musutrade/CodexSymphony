GH-115: canonical workflow body, preserved required_labels, full SHA-256-addressed workflow archives.
Related unittest suite: 12 passed. Gate config check: passed. git diff --check: passed.
Test and dev PostgreSQL fixtures running; real test database SELECT 1 passed. Python, Cargo, Node available; workspace and /tmp writable.
Initial regression run exposed a mutable mock fixture reused across installations; corrected to supply a fresh mapping per call. No host deployment performed.
Formal complete validation is requested through local_gate after this manifest is finalized; its receipt is retained by the host.
First formal Gate failed in frontend preflight: ng not found (dependencies absent). npm ci --offline --prefix web/angular then completed with exit 0 using provisioned cache. No source or lockfile change. Full Gate will be restarted.

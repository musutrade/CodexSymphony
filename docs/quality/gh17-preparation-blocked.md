# GH-17 blocked: product probe through the actual host launch path

## Confirmed facts

- Branch remains `symphony/GH-17`, local baseline `08ff122` (merged dependency PR #37). Both dependency checks, `Harness-Gate` and `Trusted Harness-Gate`, were read as success.
- The fixed `/opt/symphony-env/execution_readiness.py` operation passed, with UID 1000, Codex 0.154.0, Core 0.4.5, writable target, protected Git and hidden host credentials. It sends only the fixed `execution-readiness` action and cannot accept product commands.
- Disposable test/dev PostgreSQL checks and recreation verification passed. Gate configuration check passed.
- The actual host launcher `/home/gem/.local/share/codexsymphony/symphony/codex-sandbox` is not exposed in this workspace. Attempting to launch it gives ENOENT before any model call. Concrete reproduction and current product-probe hashes: `target/gh17-host-adapter-blocker.json`.
- The product sandbox sampler resolved both default/Cargo-priority Core entries to the locked SHA256. Its network probe did not establish a full passing boundary: direct sandbox DNS is unavailable, and the denied CONNECT probe ended without a response visible to the sampler. The outer command tool reported the expected policy denial. These are not a successful product network attestation. Saved sample: `target/gh17-product-sandbox-probe.json`.

## Saved work and validation limits

Existing operator/development-environment changes are preserved. New, unfinished product work includes preparation decisions, a PostgreSQL retry/history record, a Python app-server/sandbox probe adapter, admission checks and a storage heartbeat/latch. Runtime coding remains disabled. No PR or remote commit was created.

`target/gh17-cargo-readiness.log` records a successful locked workspace compile before implementation. `target/gh17-tests.log` records a passing workspace test run including the initial preparation tests. Subsequent storage timeout/latch edits have not been retested. The current work is incomplete and is not claimed as accepted or publishable. Format/Clippy/full local validation and exact-source host quality acceptance remain outstanding.

## Recovery and resume

Provision an operator-owned fixed acceptance operation, e.g. `/opt/symphony-env/product_preparation_acceptance.py`, which runs the reviewed product adapter and sampler through the same installed host launcher/UID/workspace-write policy. Bind its output to the exact reviewed adapter/sampler hashes and a fresh sample identity. Do not expose arbitrary host commands, credentials, or policy modification to the Agent.

Machine-checkable recovery: that fixed operation is callable and exits 0, returning a fresh host-owned result with the exact source hashes, actual UID/cwd/policy, effective network configuration identity, allowed/denied/direct-IP results, locked tool identities, and zero model calls. It must support the designated missing-dependency, wrong-capability/version and read-only-target acceptance cases without changing global network policy. An equivalent fixed host interface may be supplied with its documented command.

Resume at product-preparation-adapter integration and acceptance. Inspect the retained dirty tree first; finish the implementation and run all required checks. The passing fixed environment readiness probe alone does not satisfy this recovery condition.

## Operator resolution

The fixed `python3 /opt/symphony-env/product_preparation_acceptance.py` now exits 0 in the actual Agent sandbox. Fresh host-owned sample `c23a2e295f214f7da81d5cfebf3d5e25` includes all six cases, exact reviewed source hashes, UID/cwd/policy, effective network identity, allow/deny/direct-IP results and zero model calls. Canonical receipt: `/opt/symphony-env/product-preparation-acceptance.json`. Product integration and full quality acceptance remain outstanding.

## Subsequent integration validation

The preserved implementation was completed and validated after operator recovery.
See [GH-17 acceptance evidence](gh17-acceptance.md) for the new sandbox sample,
exact commands/results and the remaining independent host/CI boundary. This
section supersedes the earlier implementation-progress status without deleting
the original blocked diagnosis.

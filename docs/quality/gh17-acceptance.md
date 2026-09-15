# GH-17 acceptance evidence

Baseline: `08ff1222b1b9d976245edaef9132f28e2ade9cda` (dependency PR #37 merged;
Harness-Gate and Trusted Harness-Gate observed successful before implementation).
Existing implementation and operator environment repairs were preserved.

## Actual sandbox receipt

Command: `python3 /opt/symphony-env/product_preparation_acceptance.py`.
Exit 0; sample `aceebbd0c19e49e1b21a5246b10e9ce6`; retained original:
`target/gh17-product-preparation-acceptance.json` and host-owned read-only canonical
`/opt/symphony-env/product-preparation-acceptance.json`.

| Source | SHA256 |
| --- | --- |
| tools/preparation/app_server.py | de69f2b75730788e53a01ec50a7c6ac3e00f045b4d1adb21160adafda32d2253 |
| tools/preparation/sandbox_probe.py | cd522b50fef21d5c209d9e256e5c4e16670052a1614b4ce58d8c262dc7b822bc |

Both files remain byte-identical to the reviewed receipt. UID 1000, actual cwd and
workspace-write policy are recorded. Default and Cargo-priority Core resolve to
0.4.5 with SHA256 `70721282c751826ed4d57e14bd7de9516e73e833aa058d758dbd2154c0aa5e10`.
Codex is 0.154.0. Ready, missing dependency, wrong capability, wrong version,
read-only target and unknown network identity cases all ran without model turns.
Every case records allowed traffic, explicit allowlist rejection and rejected
direct-IP connections. Unknown network identity is refused.

These are fresh observations of the reviewed product adapter in the installed
host sandbox, not evidence that the future coding scheduler or remote product
is enabled. No global network configuration was changed.

## Test boundaries

Rust tests use real PostgreSQL fixtures, Git worktrees and supervisor processes.
They cover persisted 30/120-second retries, two re-probes and the ten-minute
controlled-clock deadline, late completion, stale evidence, exact launch identity,
Broker ownership, retained probe failures, user pause, ENOSPC via /dev/full,
missing storage and persistence timeout/outage. The guarded external-write
counter remains zero after a storage failure; no remote GitHub write adapter is
implemented yet. Runtime continuation integration remains a downstream task.

The independent trusted host must still collect/sign evidence and run full Gate
on the exact published SHA. Local measurements do not authorize configuration,
reuse signed requests, or constitute either protected GitHub check.

A broader diagnostic run of `python3 -m unittest discover -s tools/tests` had
11 successes and two host-only errors: the Agent sandbox denies Unix socket
creation for the replay broker and nested bwrap namespace creation. Those
unchanged host Gate tests require the existing separate trusted-host path.
Their failures are retained in `target/gh17-tools-tests.log`, not reported as
local passes or substituted with N/A. Applicable environment helper tests run
separately; no host Gate policy or threshold was changed.

## Completed local validation

- `cargo fmt --all -- --check`: exit 0.
- `CARGO_TARGET_DIR="$PWD/target" cargo clippy --workspace --all-targets --locked -- -D warnings`: exit 0 (`target/gh17-clippy.log`).
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo test --workspace --locked`: exit 0, 39 tests, including nine preparation tests (`target/gh17-tests-final.log`).
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo llvm-cov --workspace --locked --json --output-path target/gh17-coverage.json`: exit 0; real workspace tests passed (`target/gh17-coverage.log`).
- `python3 target/gh17-measure.py`: the unchanged, pinned rust-source-risk rc.2 measurement implementation consumed that LLVM export. 362 source callables mapped; **zero violations**, maximum CRAP **10**, minimum function line and region coverage each **4/5**. Retained `target/gh17-source-measurement.json` and `target/gh17-source-measurement-summary.log`; source SHA256 values were rechecked against the final workspace. This is an unsigned local rehearsal, not a protected check.
- `python3 -m unittest discover -s tools/symphony -p 'test_*.py'`: 21 tests passed (`target/gh17-python-tests.log`).
- `python3 tools/gate.py config check`: exit 0, unchanged quality configuration valid (`target/gh17-gate-config.log`).
- `git diff --check`: exit 0.

The source collector came from the checked-in rc.2 archive, verified SHA256
`aabdbbafa78b20afa3e500b58e74a132002996838b9d812e4ea15f1b193f69b8`.
The local script calls its `measure.measure` over every `apps/server/src/*.rs`,
and checks exact rational CRAP ≤10 and line/region ratios ≥4/5. No measurement series, thresholds, signed inputs, workflow or requiredness was
changed for the rehearsal; measurements used the original source positions and
complete production inventory. The full original LLVM export and collector output remain in
the workspace; the independent host recollects from the published commit.

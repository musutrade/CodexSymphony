# Gate execution and retained evidence

The reference run is PR #96, Actions `35706315708/2`: 32m18s overall,
15m23s backend capture and 8m56s ordinary backend tests. Its retained native
objects and counters occupy 4.85 GiB. Phase estimates before this change use
log modification times; new runs record monotonic durations in `timings.jsonl`.

## Execution

The installed host runs gate regressions, Rust formatting and frontend lint
before coverage compilation. Regressions are also a required `gate.selftest`
step in the final report. Each test directory runs in a separate Python process
to prevent modules with the same name from interfering with one another.

A successful complete Rust coverage capture creates `test-capture.json` outside
all project-writable mounts. It binds the exact commit, run, input inventory,
capture selection and test log digest. Final verification reads it through a
read-only mount, checks it against its source snapshot, and executes doc-tests
separately. A changed source, partial test selection, altered log or failed
capture cannot be reused. Local invocations without a host receipt execute
ordinary `cargo test`; expanded workspaces fall back to that complete command.
Neither failed tests nor old attempts are reused.

Rust subject relocation compares the complete relative-path/source-hash
inventory instead of exporting LLVM again. Final collection still hashes all
retained objects, merges native counters, exports LLVM coverage and checks the
complete subject list. Coverage and CRAP limits are unchanged. Compression
deduplicates decode/encode work while retaining distinct source descriptors and
paths; contract source bytes are not re-encoded.

## Compiler cache

Only a complete passing `push` run on `main` publishes a seed. PRs receive
independent copies, never a writable mount or hard link to the seed. The key
includes Cargo manifests/lockfiles/configuration, Rust/Cargo versions, approved
runtime digests and policy digests. Instrumented and ordinary artifacts keep
their separate Cargo directories. Only dependency outputs are retained. The
host discovers repository package names and explicit/automatic Cargo targets,
then excludes their executables, libraries, fingerprints and build outputs
before copying. Profiles/counters and incremental output are also excluded,
so each coverage run measures fresh execution. Cache schema 2 prevents an older
whole-target seed from silently reintroducing project binaries.

This distinction matters when restoring a cache requires physical copies: the first whole-target
seed occupied 8.05 GiB and took 207 seconds merely to restore. Project test
executables must not be copied when Cargo will rebuild them for a new snapshot.

The reviewed remote installer exposes `--cache-max-bytes` (default 40 GiB;
zero disables caching) and `--cache-ttl-seconds` (default seven days). Oldest
entries are evicted under an exclusive lock. Interrupted publication leaves no
usable entry; the next locked maintenance pass removes its temporary directory.
Per-run targets are still removed after success. `cache-restore.json` and
`cache-publish.json` distinguish misses, hits and capacity rejection.

The cache assumes main is an operator-reviewed branch. It never promotes PR
build output merely because that PR's tests passed. Cold runs remain supported.
An operator can migrate an existing full-PASS main seed by filtering its files
under the new schema, after verifying the old report and that only cache/host
orchestration code changed. The migrated receipt keeps the original main SHA;
no PR-generated binaries are promoted. Retain the migration provenance and
validate a complete run using the migrated seed before considering the rollout complete.

## Scheduling and post-merge verification

Actions separates host admission from execution: repository variables
`GATE_QUEUE_MINUTES` (default 60, maximum 120) and `GATE_EXECUTION_MINUTES`
(default 45, maximum 55) govern the two budgets. Only the expected GitHub App's
check for the exact commit and attempt can start the execution clock. The host's
own execution deadline remains independently configurable.

After one full main run warms the compiler cache, the operator can enable
`--reuse-identical-tree`. A main push can then reuse a full passing PR baseline
only when its complete Git tree, policy identity, installed runtime and retained
report match. The host checks report completeness and every evidence commit.
It writes a new `tree-equivalence.json` for the new SHA/attempt, explicitly saying
`full_suite_executed: false`; it never rewrites the original evidence's SHA.
Changed trees/policies, missing evidence and manual `workflow_dispatch` runs
execute the full suite. Equivalence receipts cannot themselves become baselines.

This policy establishes source-tree equivalence, not a second observation of
runtime behavior at the merge SHA. It is appropriate while build/test inputs are
the reviewed tree, pinned runtime and approved baseline. If commit metadata or
external mutable state becomes a behavior input, disable equivalence reuse and
extend the identity model before enabling it again.

## Deployment

Install an immutable host release and a complete reviewed approval with
`execution_version: 2`. Update the frontend measurement-series binding because
its capture recipe includes `tools/quality-host/capture.py`. Keep an indivisible
previous approval during rollout for already-running old source snapshots.
Repeat `--previous-config` for each older primary snapshot that is still needed;
the installer does not implicitly inherit an unbounded approval chain.
Drain the current remote job before switching the service. Do not replace the
installed HTTP authentication runtime with the older repository implementation;
carry its reviewed files forward when assembling this performance release.

Validate a cold full run, a full main publication and a subsequent cache hit.
Enable post-merge equivalence only after that full main publication. Retain the
timings, original full reports and equivalence receipt as separate evidence.

# GH-20 fixed-candidate validation acceptance

The initial PR failed trusted source inventory on inline tests, then on an
expression closure's LLVM anchor. Tests now live under `apps/server/tests` and
production validation uses explicit, independently measurable helpers.
The five historical Symphony runs and one automated repair remain recorded;
this completion is a user-authorized manual takeover, not a budget reset.

## Product behavior

- Runtime completion remains an independent `AgentRun Succeeded` fact. A host
  `RUNTIME_CONFIG.validation` plan enables the worker's validation bridge. Without
  a reviewed plan, the candidate remains pending validation, never Submitted.
- The worker restores the preserved Broker commit into a separate checkout.
  SHA/tree, deployment plan, protected entry and sandbox identity bind each
  invocation. Bubblewrap provides a read-only candidate and isolated PID/network
  namespaces; a fixed file-size limit and timeout bound command output/work.
- The plan maps contract step IDs to reviewed `/gate-entry` commands. The entry
  is an absolute, operator-owned executable mounted read-only into the sandbox.
  The deployment must provision the tools needed by its gate; executor-provided
  shell commands, credentials and repository policy changes are not accepted.
- All required steps and raw output digests must match persisted evidence.
  An empty plan, changed identity/source/entry, missing or altered output and
  unknown exit status cannot pass. A completed invocation is reconciled without
  repeating commands; an unresolved claim cannot silently launch again.
- Validation records and step output remain distinct from Run status. The
  read-only validation API includes identity, stage, result, output digest,
  retained size, timestamp and consumer. Failure is not erased by Agent success.
- Only a known code failure can reserve ordinal 1 under the shared transaction
  lock. Reservation, candidate ownership and repair intent survive restart.
  The repair worker restores that candidate, runs existing preflight, checks
  pause/revocation/storage and the original cumulative budget, then binds one
  new Run. Runtime receives the raw failure, original candidate and remaining
  acceptance plan. An old-incarnation intent requires reconciliation.
- A repaired candidate goes through a fresh validation. Success leaves a handoff
  candidate; another failure does not create another repair. This issue does
  not implement the downstream PR sender or claim business Submitted/Done.

## Verification

Tests cover real PostgreSQL concurrency and persistence, real Git Broker
preservation/restoration, real PID/network/filesystem isolation, protected
entry tampering, timeout/output limits, crash reconciliation, paused repair,
preflight admission, cumulative budget admission, one successful repair and a
second code failure that cannot consume another Run. The coding boundary uses
a deterministic fixture rather than an external model call.

Commands:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- `cargo test -p codexsymphony-server --test validation --test validation_runner --test validation_store --locked --offline`
- Installed trusted Rust source collector with real `cargo llvm-cov` and the
  complete test suite, using a fresh disposable PostgreSQL database.
- Exact-commit remote `Harness-Gate` and `Trusted Harness-Gate` remain required;
  no thresholds or signature checks are relaxed.

The one unmerged migration is revised in this PR. Local rehearsal databases
that applied an earlier PR version must be replaced, not have migration
checksums rewritten.

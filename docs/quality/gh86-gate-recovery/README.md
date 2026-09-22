# GH-86 Gate recovery

The original head `7e709d1e0a2712910cb3a59fc9de7d36236f2dc7` failed Rust
source discovery before quality evaluation. The source-risk collector did not
recognize `tokio::select!` and assumed closure entry counters always began at the
outer expression. The rc.5 candidate adapts the existing select grammar and
follows exact leading AST expression anchors, retaining nested callable ownership.
Real rustc/LLVM regressions check constructor-wrapped tuples, unary expressions,
arrays, structs, conditions, nested closures, and unpolled futures. No arbitrary
contained-region matching or parent-counter substitution is allowed.

Exact `#[cfg(test)]` modules are outside the production callable boundary; other
conditional attributes still fail closed. Internal transaction tests cover pause
barriers and retained checkout identity without racing wall-clock timing.

The project also contained pure projection closures for which the compiler emits
no independent coverage counter. Equivalent comparisons and a named retry-delay
accessor remove that unsupported form. This does not waive missing mappings in
the collector. Simple blocks were insufficient because rustfmt normalizes them.

Automatic-merge orchestration is split into query/eligibility, persisted send,
independent validation, fresh remote confirmation, and transactional completion
helpers. The original ordering of authorization checks, pause/cancel barriers,
unknown-send handling, recorded evidence, and completion commits is preserved.
Validation invocation and output-finalization checks remain mandatory.

Real HTTP/database/process regressions cover workflow dispatch idempotency,
post-merge check failure/deadline, transient observation recovery, pause during
actual merged-source validation, retained storage materials, and exact-commit
fetch failure without persisting credentials. The bounded-recovery test fixture
serializes process-global storage fault injection and uses the real recovery
path between disposable schemas.

The collector archive and digest are pinned in collector-candidates.json. A new
measurement-series identity binds the changed implementation. Coverage remains
at least 80%, CRAP remains at most 10, all policies remain required, and the API
baseline is unchanged. Local tests and diagnostic replays are not formal CI
acceptance; delivery requires fresh exact-head checks from the trusted host.

The HTTP fixture waits for the real asynchronous recovery barrier before command
replay. A listening socket alone does not authorize resume operations. The wait
is bounded and fails closed; expected HTTP statuses and the API baseline remain
unchanged. Formal attempt 35687991414/1 retained the prior startup conflict.

Formal attempt 35689596282/1 exposed two infrastructure capacity limits: the
complete backend suite takes about 329 seconds without compilation, exceeding
the prior 300-second command deadline; its deadline is now 600 seconds. The
host's lossless XZ encoding uses an extreme text profile to retain every byte
and source descriptor within the unchanged 64 MiB artifact limit. No tests,
coverage counters, required checks, or quality thresholds are removed.

Artifact capacity is now a project configuration instead of a fixed core ceiling:
`[limits] max_artifact_bytes = 1073741824` in `.harness-gate/quality.toml` sets a
1 GiB aggregate budget for this project's shared evidence directory. The core
default remains 64 MiB for configurations that omit the setting. Both `verify`
and `quality collect` apply the positive, configuration-digest-bound value.
This resource setting changes no coverage/CRAP policy or artifact integrity check.

Core candidate `0.4.6-rc.1` source: musutrade/Harness-Gate#277 (`8f1b6c1`).
Linux amd64 binary SHA-256: `6d5dcf10b8d6b1248679974664a97b29628fa24778047484f18b0ebbe5d27dd3`.

# GH-13 Rust measurement and durable blocker recovery

The HTTP rc.4 recovery remains valid. GH-13 next encountered unsupported Serde
metadata, JSON/format macros, and a lowered Result-wrapped closure entry in the
Rust source collector. The reviewed rc.2 archive addresses those cases without
changing requiredness, coverage >=80%, CRAP <=10, or source/native binding.

The source candidate has 8 AST and 12 Python tests. Fresh capture inside GH-13's
actual command sandbox mapped 68 production callables. The trivial generic JSON
extractor was rewritten as an equivalent match because LLVM emitted no separate
counters for its inline map/map_err closures; missing mappings were not waived.
Actual missing error-path coverage and execute() risk remain issue code work.
The serde_json dependency already pinned in Cargo.lock is promoted from test-only
to production use so the host approves that pipeline input before publication.

The complete local isolated CI gate passed; exact inputs and retained report are
in local-acceptance.json. This is infrastructure acceptance, not GH-13 acceptance.
Successful host runs now discard only rebuildable target caches after retaining
native objects, counters, HTTP binary, logs, source snapshots and signed evidence.
Two tests check retention and refusal to follow a cache-root symlink.

WORKFLOW.lifecycle.md defines explicit blocked handoff. The installed external
Symphony extension validates matching issue/repository, bounded reason/evidence/
recovery fields and rejects ambiguous PR fields. It uses the existing runner-stop
and durable controller-block path; blocked tasks do not consume continuation
turns or retries, including after restart. The additive host implementation patch
is retained here against the preexisting local Symphony lifecycle extension.
The targeted lifecycle/budget suite passes 33 tests, including real runner stop,
wrong declarations, persistence and restart.

The complete external Symphony `make all` also passes: formatting, lint/specs,
342 tests (0 failures, 6 opt-in skips), 100% configured coverage and Dialyzer.
The test profile was rebuilt with a stable TMPDIR to replace a stale compiled
default from a different temporary-directory path; no threshold was changed.

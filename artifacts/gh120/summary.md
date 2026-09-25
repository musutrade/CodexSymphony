# GH-120 validation and scope

The final prepared full Rust suite exited 0 and verified the source inventory did
not change during execution. The exact command result, environment/tool hashes,
and summed test-result counts are in validation-summary.json. Real pinned Runtime
integration is included in the full suite; external GitHub/cloud spikes were not run.
Formatting, strict Clippy, Gate configuration, and four real namespace boundary tests
passed. Earlier full-suite and Clippy logs are retained as development history;
clippy-final.log is the corrected needless-borrow failure, not the accepted result.

Normal validation evidence is in normal-path-verified/: frozen Call and candidate,
approved Plan input, environment contract, raw check output, Evaluation, supervisor
identity and matching quiescent receipt. retention-map.json verifies raw references
against their retained bytes. These are actual local process/Git/PostgreSQL fixture
observations, not Agent self-reported PASS, a host archive receipt, or a Gate signature.

The source inventory binds all source files and modes to baseline
93282b2d64d3bed1a7cfc1c7e8aecefc3935187b. Dependencies and remote-base readbacks are in
prerequisites.json. Migration 0033 preserves historical facts and budgets; pause/cancel
invalidation does not rewrite AgentRun or business completion. Compatible rollback
requires quiescence, preservation, and reconciliation; see docs/validation-hooks.md.

Coverage/CRAP thresholds, CI and installed Gate policy were not weakened. Complete
local_gate must validate the finalized tree before any ref/PR publication. Its trusted
receipt and eventual protected CI remain separate from these local checks. The
handoff will record the actual published SHA and Gate result; this summary deliberately
precedes that Gate invocation so no source/evidence edit is needed after PASS.

The remaining serial Issues own delivery extension integration (#121), actual
local_git delivery (#105), and two-mode product acceptance (#122). No production
deployment, Issue closure, CI polling, or merge is performed here. The host after_run
archive is not accessible to the Agent, so no archive-copy success is claimed.

Recovery: four fixed-list vec! expressions became equivalent Vec::from arrays.
Current source binding and successful repeated fmt, Clippy, configuration and full
fixture suite are in recovery-summary.json and source-inventory-recovery.json.
Earlier summaries remain historical. Complete Gate remains required.

Latest recovery: host diagnostics identified the LLVM profile output in the candidate. process.rs preserves LLVM_PROFILE_FILE across child allowlists. Current-source fmt, strict Clippy, config diagnostic and the full prepared suite passed; see profile-routing-summary.json, source-inventory-profile-routing.json and profile-routing-recovery.md. Earlier records remain historical. Complete exact-tree Gate is the next required check.

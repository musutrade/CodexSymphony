# GH-20 CI recovery

PR #46 candidate 8907b5e43f10dedd70fe1f751119198042f0b333 failed Actions
35075969342/1 during the trusted Rust collector's inventory stage.
The collector rejected the inline cfg(test) module in src/validation.rs
(unsupported cfg/test attributes and vec/assert macros).

The operator moved the three existing tests without changing assertions into
apps/server/tests/validation.rs, importing the public validation API. Production
code and collector policy are unchanged. These workspace changes are pending
publication through the existing same-PR CI repair flow.

Verified locally on 2026-09-16:
- Installed rust-source/0.1.0-rc.3/inventory accepts src/validation.rs (exit 0).
- cargo fmt --all -- --check (exit 0).
- cargo test -p codexsymphony-server --test validation --locked --offline:
  3 passed, 0 failed.

This is not complete Gate acceptance. Remaining coverage or other checks must
be evaluated by fresh trusted validation of the new candidate.

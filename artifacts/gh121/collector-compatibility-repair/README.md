# GH-121 installed collector compatibility repair

The installed rc.7 collector rejected `vec!` and Serde `default` / `skip_serializing_if` metadata before cargo coverage execution. The collector binary, policies, thresholds and excludes have not been changed.

Product commit: `1888415503edbb3325cce7e9267828133ae2247b`
Product tree: `7195d5a151ddd887cc191023f74c491e886cc0b2`

- Replaced three `vec!` expressions with `Vec::from`.
- Retained missing repository ID -> 0 and missing profile extensions -> empty through explicit decoding. Original map access preserves duplicate/unknown/null rejection, including nested duplicate keys.
- Retained profile serialization field order and omission of empty extensions, so existing frozen profile hashes do not change.
- Added differential JSON contract tests against the original derived profile decoder/serializer.
- `compatibility.json` and `source-inventory.json` prove installed collector inventory success for all 138 production Rust source files, bound to hashes. This is source compatibility only, not measured coverage/CRAP or a complete Gate PASS.
- Prepared environment suite, fmt and clippy logs and exit files are in this directory. The obsolete-launcher log records a rejected stale wrapper before test execution; current logs use the installed trusted environment fingerprint.

Real core acceptance for the repaired source uses a fresh bounded host fixture, independent database/branch, and the same reviewed validation/publication boundary. Read `../real-delivery-core-1888415/files.json` and `../host-real-core-1888415-acceptance.json`. The fresh fixture created disposable-2 PR #7 and observed pinned test-job Success; 32 exported evidence files are hash-verified. The current Agent sandbox executed the bounded submit route successfully: `python3 .agent-env/core-1888415/request.py submit`. Repeat observation is available with `observe`. Old PR #6 remains historical evidence for commit 8484905, not acceptance for this repair.

After assessing evidence, run complete `local_gate` on the current exact tree. Do not publish without its new complete PASS; compatibility and real fixture acceptance alone do not satisfy Gate. Further source changes require fresh applicable source-bound evidence. Preserve all earlier evidence and budgets.

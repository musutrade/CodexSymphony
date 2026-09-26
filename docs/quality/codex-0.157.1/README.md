# Codex 0.157.1 adaptation — validation incomplete

This directory preserves the original pre-GH-105 upgrade attempt. Its source
hashes and results describe that historical tree, not the combined GH-122
candidate. Current delivery scope and unresolved prerequisites are recorded in
[dual-path acceptance](../../dual-path-acceptance.md).

Updated the application and environment pins from 0.156.1 to the locally installed
0.157.1 binary, including its SHA-256, preparation probes, both runtime handshake
checks, scripted fixtures, real-runtime assertions and generated protocol provenance.
The edited production Rust handshake uses explicit control flow, with no closure.
Added rejection cases for the old version and a missing runtime identity.

Schema comparison used both installed binaries. Of ten consumed schema documents,
only InitializeParams changed: InitializeCapabilities gained the optional boolean
explicitGatewayOauth. The generated Rust fields are unchanged. Command execution,
configuration reads, initialization/thread/turn response schemas also remain identical.
The protocol was regenerated from 0.157.1; this is not a replacement for Gate validation.

## Source-bound measurement

The approved, unchanged rust-source rc.7 collector captured an immutable copy of
Cargo.toml, Cargo.lock, apps and migrations in the existing isolated host environment,
using an independent disposable PostgreSQL database and the 0.157.1 runtime.
It ran the full cargo llvm-cov suite, retaining native objects, counters and LLVM JSON.
The approved measure.py mapped all 2,120 callables from the 147-file production inventory.
All required per-callable line/region coverage >= 80% and CRAP <= 10 thresholds passed.
342 test pass results were recorded (including subprocess test results).
Every capture input was compared with the current workspace after measurement.
Raw artifacts remain at the path in summary.json; compressed measurements and test
logs are retained beside this document.

Changed initialize methods:

| Source | Line coverage | Region coverage | CRAP |
| --- | --- | --- | --- |
| runtime_client.rs | 17/17 | 29/30 | 5 |
| generation_runtime.rs | 14/15 | 25/29 | 676/135 |

## Unresolved validation failures

- Complete installed Gate exits before testing: `environment drift: workspace contract differs from installed host release`.
  The installed host still pins Codex 0.156.1. Only environment.lock.json differs among
  its approved trusted inputs; no Gate policy or collector change is proposed.
  The host release/approval and affected deployed environment bindings still need an
  authorized coordinated upgrade. Existing host approvals and services were not modified.
- The repository has no approved Python source-bound coverage/CRAP measurement configuration
  for the changed generator, preparation probe and scripted fixture. This missing
  measurement remains a failure under AGENTS.md; Rust evidence does not cover Python.
- Other tests and the complete Gate have not passed on this final tree. No publication,
  commit, deployment, policy waiver or lowered threshold was performed.

Existing unrelated working-tree changes were preserved.

# GH-121 real migrated-core acceptance

The previous runner limitation is now resolved. Read host-real-core-delivery-acceptance.json and real-delivery-core/files.json. These NEW receipts supersede the old baseline/native-observation-only runner for this acceptance. Product commit 8484905ebf5e5fc63f31a769f3d1a76cf52bf302, tree ff527305dcd9f875581449a79d5c0322bdec7573, is unchanged.

The new fixed host runner created a new, bounded candidate on ai/req-1-gh121-core-8484905 in repository musutrade/disposable-2 (1377749969). It uses its own PostgreSQL fixture, with synthetic Requirement/AgentRun/workspace inputs. It DOES NOT seed successful validation or fabricated delivery receipts. Product validation_service::validate executes the reviewed cargo test --locked --offline plan, retains actual logs and source/plan identities, and creates the outbox through product validation_store::finish. The test subprocess cannot see the App private key OR the verification database connection file.

Actual push and PR creation call github_publication_adapter::submit -> delivery_extension::invoke -> native GitHub adapter. Both real sends have product-persisted delivery_attempt rows, independently asserted before each send. Product observe/reconcile then confirms PR identity, records its observation and moves the fixture Requirement to Submitted. Real PR: https://github.com/musutrade/disposable-2/pull/6. The pinned test-job check is Success on candidate b6a4ca58c48abacde8015f74e3f45c3c501bd78d. A duplicate core submit did not create another intent or send. No merge, main, workflow, protection or production data write was performed.

Current executable Agent route (tested inside the actual Agent sandbox):

```sh
python3 .agent-env/core-delivery/request.py observe
```

setup verifies retained genuine validation and capabilities; submit reconciles the SAME fixed candidate/PR and cannot create arbitrary branches or commands. Approved digest is in .agent-env/core-delivery/fixture.json. Do not use the older .agent-env/real-delivery client for this acceptance. Host originals, DB intents and request receipts remain authoritative. Full source of the reviewed driver/wrapper and SHA256 are in real-delivery-core/reviewed-runner-source.json.

Scope: this is the real normal GitHub path of the migrated core under a preserved legacy-compatible frozen validation policy. Optional additional Hooks/fault cases remain covered and assessed in the issue's controlled tests; the real fixture does not claim every scenario. new_core_submit_exercised=true and real_core_publication_pass=true are now supported by actual new publication. full_issue_accepted=false / complete_gate_pass=false deliberately remain until all issue checks and the complete exact-tree Gate pass. They are not a missing-runner blocker.

Continue acceptance assessment, evidence finalization, and complete local_gate. Do not publish with stale Gate evidence. If product code changes, distinguish changed core source from evidence-only changes and retain the exact scope of these receipts.

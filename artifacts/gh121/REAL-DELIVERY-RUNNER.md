# GH-121 host real-delivery runner

User sequencing: runner first, then implement GH-121. The runner is installed and executable now. Read host-real-delivery-test-setup.json and real-delivery/setup.json before implementation.

From the task workspace:

```sh
python3 .agent-env/real-delivery/request.py setup
python3 .agent-env/real-delivery/request.py submit
python3 .agent-env/real-delivery/request.py observe
```

These are bounded host operations; they do not execute the client's Python or any workspace program with credentials. The client submits only an operation name and approved fixture digest. Results and complete sanitized logs are in artifacts/gh121/real-delivery, with sizes and SHA256 in files.json. Host originals remain authoritative.

The current frozen baseline Rust adapter has actually pushed the one fixed candidate and created https://github.com/musutrade/disposable-2/pull/5. Repeating submit reconciles this same PR; it cannot create a second candidate or branch. Observe reads real Rust product observation including the pinned test-job workflow. No merges, main writes, protection/workflow edits, task database mutations or Gate bypass are authorized.

This fixture tests the real adapter normal path. It deliberately has no fabricated successful validation rows in a product DB. It does not prove new core admission, migration, faults, or the unimplemented GH-121 changes. Implement and validate those as part of GH-121. A baseline run is NEVER GH-121 acceptance.

When implementation is ready, provide exact commit/tree plus applicable local validation evidence and request host candidate deployment/rebinding via waiting_external operation product.real_delivery_candidate_install. Host review must freeze the driver and candidate, rebuild without credentials, and install the reviewed binary before the new candidate real regression. Do not execute candidate-supplied scripts with App credentials, downgrade gates, use github_api as a substitute for the Rust adapter, or infer acceptance from this readiness receipt. The existing credential-isolated runner and designated scenario remove the current missing-runner setup blocker; candidate rebinding follows implementation.

The full exact-tree Gate PASS and controlled regression suite remain required before publication.

This new host fixture is ONLY for authorized external real GitHub delivery. Ordinary Cargo/Runtime tests still run directly in the existing prepared environment; do not reinstate obsolete reviewed-binary requirements for local development.

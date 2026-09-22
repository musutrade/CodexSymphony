# Reviewed Gate runtime and lossless evidence packaging

GH-86 (PR #95) merged with the configurable 1 GiB artifact budget and the
reviewed `harness-gate v0.4.6-rc.1` binary. Its complete trusted host run was
`run-ab823604657a`; GitHub Actions run `35693426415`, attempt `2`, and trusted
check `106640762761` passed for head
`3f0bbfe0d67f1753c97181af397d9a97c16f12cd`.

The host already runs the fixes persisted here. This source update does not
reinstall or restart that host or the active Symphony controller.

`tools/symphony/reviewed_gate.py` selects the binary named by the workspace's
`harness-gate-version.lock` from an explicit version/digest allowlist. Unknown
versions and altered installed binaries fail. Both the workflow preflight and
the trusted worker environment use this selection. The development installer
copies the helper into the immutable worker release and the stable workflow
hook location; helper changes participate in the release digest. A new Gate
version requires review of its installed binary and an allowlist update.

The signed host launcher losslessly packs backend/frontend coverage artifacts
with XZ when this reduces their size. Every source retains a distinct artifact
path and descriptor. Content, source/context bindings, metric references and
counter populations are preserved; nested references and input hashes are
validated before outputs are changed. This supplements the project-configured
aggregate byte budget; it does not waive the budget or quality requirements.

API contract artifacts are deliberately excluded from reencoding. Their raw
artifact digest must equal the source digest used by cross-component contract
validation. The launcher regression checks this boundary explicitly.

Validation commands:

```sh
python3 -m unittest discover -s tools/symphony -p 'test_*.py'
python3 -m unittest discover -s tools/tests -p test_artifact_packaging.py
```

Changes under `tools/quality-host` alter the reviewed host-source inventory.
Before deploying this source revision, use the existing host review/provisioning
process to produce the matching approval. Do not edit an immutable installed
release or remove an active Issue's approved snapshot during the transition.

For a rolling transition, `tools/install_remote_gate.py --gate-approval NEW
--previous-config OLD --dependency-source NODE_MODULES` installs a bridge which
accepts the current and one explicitly retained previous deployment. Each
candidate includes its complete protected-file inventory and a separate host
approval. Selection never combines pins from different candidates; unknown or
mixed source snapshots fail. Each run's policy fingerprint uses the selected
snapshot, and selection does not mutate the service configuration.

Keep the previous deployment while existing Issues still use its source
snapshot. Once those Issues finish or rebase, reinstall without
`--previous-config` to retire compatibility. The gate host and worker releases
remain immutable; retaining their reviewed runtime is appropriate when this PR
only persists fixes already deployed there.

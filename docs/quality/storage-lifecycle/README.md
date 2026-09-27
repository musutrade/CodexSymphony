# External capture cache repair

The user authorized task-scoped coverage.py/Radon validation for the changed Python
sources. All 26 measured production functions have 100% function-body line coverage
and CRAP <= 10; exact ratios, source hashes and tool versions are recorded in
`python-measurements.json`. This does not approve a permanent Python collector.

Subsequent retention, storage and archive regressions passed 75 tests. Raw coverage,
capture logs, registration inputs, manual cleanup receipts and final Gate evidence
are retained outside the workspace under
`storage-maintenance/operations/20260927-storage-lifecycle/`.

Existing installed safety checks reclaimed 29,116,010,496 bytes of completed manual
capture compiler caches after verifying independent raw evidence, and the approved
compiler-cache API reclaimed 13,578,454,644 bytes of dependency seeds. These are
operation counts, not a claim about later disk free space. Incomplete captures and
all original raw payloads remained protected.

The automatic fix adds explicit completed-capture registration and hourly cache
collection without an extra idle delay. It also fixes padded SSH session titles
that previously deferred collection despite having no build workers.
See [manual capture lifecycle](../manual-capture-retention.md). The installer is
not deployed until the final exact-tree complete Gate passes. Report that Gate
identity and deployment status from the host receipt; never infer either from this
tracked document.

The release candidate also binds retention script hashes, service and timer files,
persistent admission drop-ins, and effective systemd commands to a host deployment
record. An independent watcher and the disk guard check this record every 15 seconds and pause dispatch
on drift; admission checks also block starting managed services with an overwritten
release. Reinstalling an old retention installer cannot silently refresh this record.
This does not authorize automatic tool approval or trusted host upgrades.

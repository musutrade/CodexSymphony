# Product storage lifecycle (Phase 0a)

`STORAGE_CONFIG` names a deployment-owned JSON file. It is required when
`RUNTIME_CONFIG` enables execution. Configure it before accepting work. It applies
only to product execution and archive roots; development evidence, this checkout,
and Harness-Gate archives are not product Run directories.

The document contains `policy`, `execution`, `cold`, `database_filesystem`, and
`database_extras` (an array of backup/log roots outside PostgreSQL's own accounting).
Every root has an absolute canonical `path` and `identity: {"device": N, "inode": N}`
from the provisioned filesystem. The service verifies these
identities before scanning, admission and archive creation. Missing or changed
mounts stop admission; they do not create fallback directories. Execution and cold
roots must be disjoint. PostgreSQL must permit `pg_database_size`, relation-size
queries and `pg_ls_waldir`; `database_filesystem` must represent its actual storage
filesystem, not an unrelated controller disk. Register externally retained WAL,
logs and backups in `database_extras` without counting the same paths twice.

`policy` requires a new immutable `version` and nonempty change `reason`, positive
finite `global_bytes`, `control_bytes`, `run_bytes`, `requirement_bytes`,
`entry_bytes`, `entry_count`, and all five `categories`: `workspace`, `hot`, `cold`,
`database`, `record`. Each category requires `bytes`, `seconds`, and `reserve_bytes`.
Sizes are bytes and retention durations are seconds. Choose values for the actual
deployment; there is no unlimited default. Per-category reservations must fit the
Run, requirement and global budgets together. Runtime/preparation/validation raw
output is capped at the smaller of `entry_bytes` and 1 MiB. Existing attachment
chunk and count caps also apply. The entry-count bound prevents unbounded scans;
exceeding it preserves files and blocks further work until reconciled.

The worker scans every 300 seconds and receives PostgreSQL notifications at Run,
preparation, validation and delivery phase completion. Admission also attempts
eligible cleanup first. Cleanup takes no execution slot and makes no model calls.
Failed scans also wake at their persisted retry deadline without waiting for a
new phase event. Their bounded, redacted cause is retained in `scan_retry` and
the service log. Accounting measures live sockets without opening them and may
restart a whole pass up to three times for disappearing temporary entries;
archive/deletion inventories remain strict. Other errors still stop admission.
Decision records, retrospective logs, recovery snapshots and rebuildable caches
have separate material identities and retention facts. Existing attempts are
inventoried even without a final report. Unclassified files and directories are counted
and displayed as classification/preservation work; they are never deleted.

Retry consolidation uses explicit recovery/repair predecessor links, repository,
requirement revision, Run sequence, candidate, stage, trusted policy and PR
identity. A later failure or unrelated PR does not resolve earlier work. Current
successful retrospectives remain protected. Active processes, pending validation
or delivery, unresolved restore intents, unknown preparation owners, partial
archives and unique uncommitted/unpushed work remain protected. A preparation
without a durable quiescence receipt needs reconciliation even if its process is
no longer visible.

Before removing hot originals, the service verifies every compressed file and
persists the package identity, digest manifest, key-log excerpts and deletion
intent. Retrying an interrupted deletion verifies each remaining file against the
original manifest; a renamed/replaced Run directory is not adopted. Empty retired
directories remain as identity tombstones. Partial archive output is protected and
charged to capacity until an operator reconciles it; failed output never replaces
an existing verified package. Historical result and exact identity remain
queryable when raw files expire. This is a retrospective, not full replay or
current validation evidence. Required live validation consumers prevent expiry.

Capacity includes filesystem blocks, PostgreSQL database/index/WAL size, configured
backup/log roots, retained summaries, outstanding background reservations and the
control-plane reserve. Reclaiming files frees occupied capacity but does not refund
cumulative Run/requirement allocations. Policy versions, intervening retries and
requirement revisions do not reset that history. Actual writes above a reservation
are recorded and block further work. Existing materials are adopted into accounting
without retroactively deleting protected originals. Expired critical decision
records remain intact and block new work until exported or their retention policy
is explicitly extended.

The operations page shows measured capacity, measurement time, reservations,
protection reasons, material expiry/deletion, replacement references and cleanup
failures. Cleanup uses the existing initial attempt plus two retries (30/120 second
backoff, ten-minute group budget), with durable attempt intent. Exhausted groups
appear in the existing inbox. After repairing storage/permissions and reconciling
partial material, `storage_recheck` authorizes a new bounded group, remeasures
capacity and clears the storage latch only if processes are quiescent. It does not
unpause the requirement or alter business, Run, validation, delivery or model-budget
facts. Original cumulative cleanup attempts remain visible.

This is admission control and periodic measurement in a trusted environment.
It does not provide hard byte isolation for arbitrary native processes. Configure
and verify a suitable filesystem/execution adapter before making that claim.

迁移 0013 将阶段通知改为实际变更行触发，空更新/未改变的状态不再触发完整扫描；
周期扫描和存储失败停止规则仍有效。控制锁的单次等待仍为 500ms，只在业务操作开始前
对 PostgreSQL lock-not-available 有限重试两次，每次新事务；其他错误直接失败，业务写入不重放。
迁移 0014 保留已准备但未派发的恢复任务历史。新控制实例重新准备新身份，原任务及文件保留；
已存在 agent_run 的任务不能走此路径。仅本地提交、没有结束声明/验证记录的中断可恢复执行；
已接受的完成声明和独立验证继续使用原阶段恢复。

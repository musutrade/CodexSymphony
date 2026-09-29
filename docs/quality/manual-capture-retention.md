# Registered capture cache lifecycle

> 当前保留策略统一从 [存储与证据保留策略入口](evidence-retention.md) 读取。本页只说明采集登记及缓存回收前提；固定宿主复用验证槽位和 target，每次仍生成独立捕获记录。原始材料在登记时完整保留，不等于永久保留；完成后的到期回收由适用的已安装策略决定。

Manual captures outside `gate-host/runs` must be registered with the host retention
tool after the operator confirms the task/capture is finished and its source-bound
capture bundle and independent raw inventory exist:

```sh
python3 <installed-retention-release>/capture_cache_retention.py --register <capture-root>
python3 <installed-retention-release>/capture_cache_retention.py
```

Registration is explicit; directory names and glob matches never authorize removal.
The host registry binds the canonical capture directory's device/inode and exact
bundle SHA256. A replaced directory or changed bundle requires operator review.
The hourly `codexsymphony-capture-cache.timer` removes only each registered capture's
`target`, without an additional idle delay for these explicitly completed captures. Ordinary
debug caches retain their existing idle policy. Before deletion,
all raw files recorded by the Rust collector must still exist independently and match
their original SHA256. Missing, corrupt or symlinked evidence prevents collection.
Worker detection protects the whole capture, repeats before removal, and uses the
existing bind-mount aliases. Operators must never resume an old registered capture;
create a fresh capture directory for each attempt.

Source snapshots, capture bundles, raw counters and objects, measurements and logs
remain at their original paths. Unfinished captures with no complete raw inventory
cannot be registered; their retained data requires manual review. This cache policy
does not bound raw evidence growth or replace the existing Gate raw-retention policy.
It exposes errors in `storage-maintenance/capture-cache-retention.json` and makes the
service fail rather than silently claiming cleanup. Registry state stays outside
task workspaces under the existing maintenance lock.

The installer creates a content-addressed release and enables the new timer without
starting it. Source-bound measurements and the final complete Gate must pass before
deployment or starting cleanup. Follow [AGENTS.md](../../AGENTS.md) for validation.

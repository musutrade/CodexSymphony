# 开发盘回收修复

仓库要求所有代码修改先通过源码覆盖率和 CRAP 测量。现有已批准采集器没有 Python 支持；本次按用户授权采用任务范围内的替代验证：先对新增及修改的 Python 函数取得绑定精确源码的 coverage.py 覆盖率与 Radon 复杂度，计算 CRAP，再执行回归和最终完整 Gate。这不是 Python collector 的长期批准，不改变仓库规则，也不使用 Rust/TypeScript 结果代替 Python 测量。

## 已执行的空间回收

2026-09-26 已按用户授权清理两处闲置、可重建的 Cargo debug 产物：共享 `cargo-target/debug` 和 `gh88-product-acceptance/build/target/debug`。检查了开发盘 bind mount 的两种路径及活动进程，使用现有已安装代码的路径安全检查；保留 LLVM 原始覆盖率、验收证据、源码和 release 产物。

操作清单与回执位于宿主 `storage-maintenance/operations/20260926T070046Z/`。清理后开发盘可用约 114 GiB，使用率从 82% 降至 73%。这些是现场快照，其他任务仍可能改变占用。

## 变更

- `retire_pr_attempts.py` 将已迁移的 Gate 软链接视为不受本地回收器管理的记录，不跟随或删除软链接及目标。这样的成功记录不能授权清理同 PR 的其他尝试；其他 PR 的正常回收可以继续。路径越界、提交或回执身份不符仍拒绝。
- `cache_retention.py` 只管理 `policies()` 明确列出的可重建 debug 缓存。容量预算和最小闲置时间只在该代码维护。超预算且闲置满指定时间时，路径检查及活动检查通过后才回收；活动或近期使用的缓存延期，不能为了容量目标删除运行中的构建。
- `install_evidence_archive.py` 将脚本装入按内容摘要命名的 release，并注册每小时执行的缓存回收 timer。安装后仍需验证再启动 timer；不覆盖原有 release。
- 维护结果写入 `storage-maintenance/cache-retention.json`。单个缓存错误保留详细结果并使服务失败，避免静默失效；正常的活动延期与故障区分。

这是闲置缓存容量治理，不是磁盘硬配额。历史验收原件不在自动删除名单中，新目录也不会因名称相似自动纳入。现有磁盘压力保护继续独立生效。

## 验证与部署约束

遵循根目录 [AGENTS.md](../../AGENTS.md)。本次 Python 测量覆盖 15 个新增或修改函数，函数体行覆盖率均为 100%，最大 CRAP 为 8（要求覆盖率至少 80%、CRAP 不超过 10）。采集执行 22 项测试；随后独立执行缓存、安装、PR 回收、存储和归档回归。原始覆盖率、分支结果、源码摘要、测试日志和测量工具的下载摘要保存在开发检出的 `.harness-gate/reports/storage-retention/`，最终完整 Gate 与部署结果由宿主操作回执记录。

CRAP 使用精确有理数计算 `CC² × (1 − coverage)³ + CC`。Python 测量只为这批 Python 改动提供证据；完整 Gate 仍须对最终树重新执行其既定的测量及所有检查，不借用历史 PASS。本任务未修改 Rust 或 TypeScript 生产代码。

只有验证完成，才运行安装器并启动归档与缓存 timer。部署后回读实际 ExecStart、版本摘要、回收日志和磁盘容量；历史软链接及验收原件必须保留，活动目录必须仍被跳过。

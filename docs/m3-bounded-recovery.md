# M3-2 有界恢复（GH-85）

基线为 GH-84 合入后的 `443404087ff38b44a1054d79d13c83558009e073`（PR #93）。本项接通合并前本地验证、必需 CI 和可执行 Review 检查；合并后/集成关联修复的入口由 M3-5 接线。没有启用自动合并、部署或扩大旧任务授权。

## 授权与持久化

新仓库登记默认选择 `bounded_v1`，整组评审按仓库快照显示每项三次及整组上限之和。已有仓库策略、已评审任务的授权保持不变。`repair_authorization` 按 Requirement 身份冻结；旧修订迁移为一次，资源加额不增加修复 ordinal。组版本变化沿用既有变化评审与累计预算，不通过新 Run、修订或移除重加重置历史。

协调器锁下，同一事务持久化失败事件、连续 ordinal 的预留、首次模型调用资源、工作区额度及具体启动意图。唯一约束阻止同一验证失败重复收费，也阻止同项两个待启动/运行中的修复。资源预留计入项和父组余额，首次模型调用转移预留，随后由原有 model_call/usage 台账累计实际结算。转移前再次检查父组与项暴露额；迟到 usage 仍归原调用。额度耗尽保留 owner、工作和待办。

冷启动先经过现有恢复屏障。仅在不存在已派发 Run 时才重建过期 incarnation 的启动意图，保留原意图及相同 ordinal/资源；存在 Run 的未知启动结果沿现有 Runtime 对账路径处理。暂停恢复产生新 Run 时，在同一事务中保留 `repair_run_history` 并转移当前修复关联，沿用 ordinal 与首次调用预留；迟到回调仍绑定旧 Run。暂停、取消、撤权、存储阻断和旧回调不得新开模型调用。

## 事实分类与重试

分类输入保留步骤、命令、输入/环境身份、候选 SHA、PR head、原始日志和日志路径。服务不可用/连接失败/限流优先于断言文本；配置、权限、安全和无法识别的诊断进入待办。仅已授权可执行检查的原生编译器/断言诊断能触发代码修复；模型建议和普通 Review 文字没有此权限。

基础设施最多两次重试，30/120 秒退避、十分钟阶段时限，遵守 Retry-After。等待不调用编码模型，也不消耗代码修复次数。本地每次重试新建关联验证记录，原始失败及固定候选身份不被覆盖。GitHub Actions 仅在仓库已授权 `rerun_actions` 时发送 rerun-failed-jobs；写前记录 unknown，丢失响应先查询 run_attempt，无法确定时只对账直到时限。明确 429 拒绝可以在退避后再次查询并有界重发。没有可核对远端 attempt 的检查不会盲目重复写入。

同输入、环境、命令、候选和失败指纹不构成新进展；修复 Run 产出原 SHA 时立即停止。后来可读的原生日志可产生关联新证据，旧缺口记录保留，新证据仍受相同代码/资源预算约束。

## 修复与 PR 更新

修复上下文包含原始失败、剩余 AC、ordinal/上限、项/组预留前余额及本次预留资源。连续修复失败以关联后继失败接续待办，原始记录保持原文；基础设施重验成功可以完成原修复。CI/Review 修复预留前要求新鲜且匹配的当前 PR head。继续复用 Runtime、固定候选验证和 GitHub Broker/outbox。已关联 PR 的修复候选复用原 PR/分支；更新要求候选为预期 head 后继，Git push 使用精确 expected-head lease，外部变更导致冲突。丢失响应先读远端，确认新 head 后才替代旧 delivery。取消尚未发送的修复更新可撤销新意图；未知发送保留对账责任。

Gate PASS、Agent Succeeded、PR merged 各自仍只是独立事实，不单独写业务 Done。失败/未知待办显示在操作详情及收件箱，包括日志原文位置。

## 迁移与回退

`0026_bounded_recovery.sql` 增加授权、失败、重试及旧启动意图记录；修复主键改为 `(requirement_id, ordinal)`，加入资源预留字段；验证增加 `retry_of`；delivery 增加原 PR 身份/预期 head/替代关系。迁移不改既有修订内容，不给旧任务增加次数。

先备份数据库与工作区并停止协调器，再应用正常迁移。禁止在同一数据库上直接启动旧二进制：旧实现假定每项只有一行 repair_reservation，且不认识资源预留与 PR 更新台账。回退须停止所有写入、保全/对账在途 Run 和远端意图，恢复迁移前数据库及匹配工作区备份后运行旧版本；若要保留新数据，需要另行评审的向前兼容修复，不能删记录或重置额度“降级”。

## 复跑入口与证据边界

使用一次性 PostgreSQL fixture，通过 `/opt/symphony-env/run.py` 执行：

- `cargo fmt --all -- --check`
- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo llvm-cov --workspace --locked --json --output-path artifacts/gh85/rust-coverage.json`
- `python3 tools/auth_browser_acceptance.py`（HTTPS、桌面/移动端与 axe）
- `python3 tools/auth_contract_acceptance.py`（真实 HTTPS 响应与凭据安全合约采集）

前端使用 `npm run lint`、`npm test -- --watch=false`、`npm run build` 和 `tools/probe-typescript-risk.cjs`。API 启动测试、浏览器与合约采集应串行运行：真实进程共享单协调器锁。测试夹具也保留真实存储故障闩锁；全套重跑前可重建 disposable test fixture，不能重置生产数据库或 synthetic dev fixture。

`bounded_recovery` 测试使用真实 PostgreSQL、并发事务和独立子进程重连；本地恢复测试使用真实 Git 固定候选与故障脚本；远端测试使用受控 HTTP 故障和持久化 outbox。它们不宣称实际 GitHub App 写权限、公网 CI 或生产部署边界已通过。最终证据清单为工作区 `.symphony-evidence.json`；精确发布 head 的两个受保护 Gate 由独立宿主完成。

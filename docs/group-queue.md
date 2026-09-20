# 组依赖队列（M1-04）

一次整组授权后，现有 Runtime 调度循环将授权快照中的代码子项投影为 Requirement，复用同一协调器、全局 `execution_control` owner、准备检查、独立 Run/worktree 和控制入口。父项和 `validation_only` 不创建编码 Requirement/Run/PR。尚未配置 Runtime 的部署显示等待调度；授权本身不启动模型。

队列按组授权入队时间与旧独立需求创建时间排序，组内按已评审顺序。不能越过暂停、未完成依赖、无效授权、仓库不可用或尚未实现的 validation_only。代码项领取前重新检查父/评审 revision、仓库策略与可用性、准备结果以及父组/子项预算；领取、Run、owner、Running 在同一事务落盘。准备意图和精确依赖输入先持久化，失败回滚不产生半个领取，重启不会分配另一个初始 Run。

当前受控 Runtime 配置提供精确仓库基线。同仓前序完成提交必须是该基线的 Git 祖先，否则等待更新基线；不会用旧基线开始后继。跨仓依赖保存来源仓库、精确合并版本、产物引用、证据摘要与适用验收计划，不进行跨仓 Git 祖先比较。版本事实与当前项的授权验证计划一起保存在不可变领取输入中。M1 不新增远端抓取/自动合并或通用工作流。

Agent 退出、PR 发布、CI 通过和单纯确认合并都不释放组代码项。旧独立需求继续沿用现有 0a 合并释放规则。组项还需要通过受控完成事实接口确认适用的子项验收；没有该能力时保持占用并明确等待。暂停/恢复和取消沿用子 Requirement 的现有操作：暂停重启不自动恢复；取消按原有收尾对账释放 owner，取消不满足依赖。父项始终等待后续整体验收，不能由 PR 数量推导 Done。

组预算在每次调用前同时检查项/组的已用量与在途预留，按原有累计模型事件归属到稳定子项和父组账本。重复事件、重启和未知用量不归零；迟到超额用量停止同组后续工作。旧单需求预算增加入口不能扩大组授权。组投影后普通草稿/评审编辑拒绝，避免改变已绑定输入；M1-05 再实现受控组变更，当前不会静默迁移旧 Run 或重置预算。

## 受控完成事实协议

`group_dependency::Fact` 定义 requirement / authorization / child revision / 本地与 GitHub 仓库身份 / PR / head / merged / acceptance SHA、完整批准验收计划、可信来源、证据 SHA-256 和产物引用。`group_completion::record` 是平台内部适配器接口，必须提供实现 `Verifier` 的受控验收生产者；不向 HTTP 或 Agent 开放写入。

入口同时验证生产者对来源与适用性的认证、精确授权绑定、非取消 Submitted 子项，以及已有观察器的非过期精确 PR 合并事实。验收必须覆盖合并版本；同一身份重复写幂等，冲突拒绝。M1 **没有配置线上 Verifier**。测试中的 `FixtureVerifier` 仅认可固定合成收据，证明接口和队列协议，不代表 M3 的真实全链路验收，也不生成线上完成事实。

## API 与界面

GET/PUT `/api/drafts/{id}/review` 在原响应增加可选 `execution`：全局 owner、全局暂停、子项完成数/总数、父项等待状态，以及按顺序排列的 child/kind/repository/depends_on/Requirement/状态/占用/完成/等待原因。`scheduler_available=true` 表示调度代码已实现，不证明部署已配置 Runtime 或业务验收能力。授权快照保持历史原值。

评审页面显示这些字段，并链接到子 Requirement 的既有暂停/取消/恢复操作。`waiting_validation_only_execution_not_implemented` 明确表示尚未实现执行能力，不能创建空编码 Run/PR。无授权或 Runtime 未调度时仍可查看组的顺序、依赖和版本计划。

## 验收证据映射

| AC | 可重复检查 |
|---|---|
| AC01 | `group_queue::global_claim_restart_budget_and_completion_protocol`：真实 PostgreSQL 并发 CAS、父项无 owner、Run 不重复；`cross_repository_versions_do_not_use_git_ancestry_and_share_legacy_owner`：跨组/跨仓与旧单需求共用 owner；原 `execution`、`multiple_repositories` 回归 |
| AC02 | `unauthorized_stale_unavailable_cancelled_and_unsupported_never_claim`：过期/撤销/仓库不可用/暂停/失败/取消/依赖拒绝；完成协议测试：非合并和缺适用验收均不释放，错误身份/计划拒绝；旧 `delivery`/`github` 覆盖 PR 与 CI 观察独立性 |
| AC03 | 完成协议固定夹具测试与跨仓测试：受控来源、精确版本/计划/产物、同仓缺失祖先拒绝、跨仓独立历史接受；不代表 M3 产品验收 |
| AC04 | 在 initial_run 写入和领取状态更新处 PostgreSQL trigger 注入失败，断言全事务回滚；新 pool/新 incarnation 保留 owner、暂停、输入和 Run 数；`group_ceiling_inflight_reservations_and_late_usage_survive_restart` 覆盖预留、重复/迟到事件、组上限与旧入口不可绕过；原 Runtime/预算/恢复测试 |
| AC05 | `group-review.spec.ts` 单测和真实 API Playwright desktop/mobile：排序、依赖、owner、父子进度、明确未实现能力、键盘、axe、减少动效、强制颜色及无页面横向溢出；后端断言 validation_only 无编码 Requirement/Run |

本次命令、实际结果和限制保存在 `docs/quality/gh63/README.md` 与工作区 `.symphony-evidence.json`。精确发布提交双 Gate 由独立宿主执行。历史 A13 缺失原件继续保留在原证据恢复索引，不以本次夹具补造。

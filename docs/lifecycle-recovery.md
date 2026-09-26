# 固定生命周期与插件恢复（#128）

本增量复用现有需求、Run、工作区、验证、交付与预算记录。系统负责推进、调用、冻结身份、权限、故障记录和验收；插件负责已授权的具体执行和通知渠道。系统不接受插件自行追加阶段、授予 PR 权限或声明需求完成。GitHub 是已接入的交付适配器；本增量不实现 #105 的 local_git 执行，也不伪造本地项目的 PR/CI 事实。

## 工作区和并发边界

保留多个工作区不代表并行调度：当前实例仍使用全局单需求 owner 和现有暂停锁。不新增多执行者租约或并行调度。工作区身份、Run、需求修订、候选 SHA/tree、验证 invocation 与批准计划共同约束结果。恢复授权与后继验证记录创建均复查当前权限；旧版本结果不得恢复取消的任务。共享插件升级不刷新其他需求的验证事实；每个重验决定绑定原 validation 和新计划摘要。

通知可并发消费不同需求。每个插件内，同一需求按 sequence 顺序交付；一条未确认通知不堵住其他需求。数据库登录只允许领取本插件事件及确认对应 attempt，不能修改需求、验证或预算。

## 门禁反馈

旧验证脚本保持退出码协议。部署批准的 step.command 第二个元素为 `--symphony-feedback-v1` 时启用结构化反馈；它属于被冻结的计划身份，候选代码不能自行切换。当前 runner 合并 stdout/stderr，故合并输出必须是一个不超过 64 KiB 的 JSON 文档，诊断详情写受控日志，不在协议输出夹杂文字：

```json
{"protocol_version":1,"check_id":"quality","verdict":"unknown","fault":{"class":"unsupported","code":"collector.closure","message":"当前采集器无法完整测量此类闭包","owner":"plugin_maintainer","scope":["quality"],"resume_condition":"批准支持该源码的采集器后，对同一候选重新验证"}}
```

verdict 为 pass/fail/unknown。pass 和 fail 都要求执行完整退出 0 且 fault 为空；fail 表示完成检查后判定代码不符合要求。能力不足、协议矛盾、证据缺失是 unknown，不自动归因代码。原始退出码、输出摘要、日志引用与 verdict 分开保存。fault.class 支持 input/resource/internal/dependency/unsupported/protocol/unknown。未知执行先确认原进程停止，不能直接再启动一份。

## 两条恢复路径

受登录及 CSRF 保护的 `POST /api/requirements/{id}/extension-recovery` 接收 request_id、version、revision、validation_id、reason、action。重复同一请求返回原结果；复用 request_id 改内容或使用旧版本均拒绝。响应 accepted 不表示已经启动。

- `action.kind=revalidate`：提供部署批准的新 plan_digest 和 resume_condition。核心创建新 validation generation，保留原失败和候选 SHA/tree，沿用 required checks，执行环境检查、before_run、验证及 after_run。不会启动 Agent 或重置预算。原验证标为 superseded，不能拿旧 PASS 交付。
- `action.kind=revalidate_delivery`：仅用于尚未交付的 GitHub 候选，额外提交 `policy_digest`，明确批准同一新计划用于本次候选重验和实际合并版本验收。摘要是当前强类型 `github::Policy` 的 `serde_json::to_vec` 字节的 SHA-256，包含原 post_merge 固定计划及保护规则；不接受任意文本作为授权。仓库版本必须与冻结修订一致，post_merge 必须原为 fixed_validation。新验证成功后，仅它对应的任务、修订、候选和验证 ID 可使用新计划。执行时复查原策略摘要、新计划摘要和保存的 trusted identity；普通 revalidate 不改变 post_merge。仓库全局策略、CI 来源、合并权限、保护规则及旧计划文件均不更新。
- `action.kind=adapt_code`：提供非空 constraints，沿用现有代码修复 worker、次数和累计资源授权，生成新的 Run 和候选；不把原 unsupported 改成代码错误。额外冻结的约束在每次 Agent 输入构造时注入，包括恢复与修复。候选修改路径必须在批准范围内，超范围阻断验证。

旧退出码适配器若留下 `blocked` 验证和 `unknown` step，但没有结构化故障记录，显式恢复事务会从原 validation_step 补记 unknown 事件。它保留原退出码、输出摘要和日志引用，不把无输出的退出 0 当成 PASS，也不归因代码错误。版本、权限、进程静止、预算和恢复次数任一检查失败时，补记和决定一并回滚。

此恢复不需要新数据库迁移；部署仍须先暂停、确认原调用停止并保留数据库及配置。`revalidate_delivery` 一旦被接受，旧二进制不理解它的合并后计划绑定，不能直接降级继续推进同一数据库。回退应保持暂停，保留新旧决定和实际副作用，再显式审核后续处理。

每个需求最多接受三次显式恢复决定，包含失败的授权；自动修复次数仍受原策略限制。部署计划摘要不匹配且尚未生成后继时，可以提交新的版本化决定；历史决定保留在 business_request。已生成后继则针对该后继的失败处理，不能覆盖旧 generation。暂停、取消、仓库撤权、未静止进程、未处理 blocker 和预算耗尽继续阻断。

约束字段为 id/version/source/reason/instruction/paths/code_scope/release_condition，作为评审合同和组子项的可选 development_constraints。paths 是仓库相对的精确文件或目录前缀；不接受绝对路径、父目录或 glob。code_scope 声明 changed_production/changed_tests/specified_files；核心检查路径范围，生产/测试语义以及“不要使用闭包”等代码规则由既有门禁实际检验，不能仅靠提示词声称通过。解除条件是评审材料，不会因插件版本变化自动删除约束。

## 生命周期通知

业务变更与 lifecycle_event、notification_delivery 在同一 PostgreSQL 事务提交。回滚不发事件，不用轮询最终状态来猜测中间步骤。覆盖需求状态/暂停/取消、环境准入结论、准备结果和重试、Run 状态/停止、问题与答复、工作区操作/保全、验证、恢复决定、交付及合并验收、四类 hook 的实际状态。同阶段环境准入结论或实现摘要改变时形成事件，相同状态的重复观察不刷屏。事件只包含筛选的结构字段，不包含代码路径、合同文本、问答内容或原始日志。

核心 `apps/notifier/dispatch.py` 按固定已批准 argv 调用插件，stdin 是带 protocol_version/plugin_id/event_id/attempt/requirement_id/revision/sequence/phase/source_id/facts 的 JSON。插件 stdout 返回：

```json
{"protocol_version":1,"event_id":123,"status":"accepted"}
```

accepted 表示插件持久接收，不表示用户已收到渠道消息。插件可返回 ignored 或 failed；超时、非法输出和响应丢失保留 unknown。固定十秒执行上限；插件不继承数据库或宿主凭据。调用前验证已登记执行文件摘要。通知不获得业务推进权限，通知失败不会重跑 Agent。

默认三次调用和十分钟总截止；重试保持 event_id，另增 attempt。迟到旧 attempt 的 ack 不覆盖新 attempt。投递是至少一次，插件按 event_id 去重。`GET /api/requirements/{id}/lifecycle?after=N` 返回最多 100 个事件和各插件交付状态；after 是该需求的 sequence。`GET .../extension-recovery` 显示原始故障与恢复决定，并沿用运维脱敏。`POST .../notifications/replay` 的 event_id/plugin_id 只能把已失败事件扩展一次到最多六次，保留全部历史和已用次数；重复请求不再扩展。

部署时以迁移 owner 创建专用通知登录，只授予当前安装 schema 的 USAGE，以及 notification_claim(text)、notification_ack(text,bigint,integer,text) 的 EXECUTE；在 notification_plugin 登记 id/database_role/enabled。不要授予业务表读写权限。dispatch 配置必须为属主私有普通文件，字段 enabled/plugin_id/database/psql_program/argv/implementation；implementation 映射绝对解释器及脚本路径到 SHA-256，脚本放 Agent 和候选挂载之外。`deploy/m2/notifications.json` 默认禁用，配套 service/timer 必须由部署安装并启用；仓库文件存在不代表线上已经启用。

Bark 以 `--event` 接收，持久化自己的 SQLite inbox 后确认；原 Bark timer 负责渠道发送和有限重试。event_policy=actionable 只发送需操作事件，all 包含进展和完成；选择在插件中。事件模式配置不包含 database/psql_program，Bark 不需要业务数据库访问。保留 enabled/application_origin/endpoint/device_key/state_directory/local_fixture，加 event_policy。旧轮询配置仍兼容，但同一渠道部署应选定一种模式，避免双发。

## 验证范围

extension_lifecycle 集成测试使用真实 PostgreSQL、Git 工作区、验证脚本及实际 Codex app-server；模型响应来自受控本地 SSE fixture，验证适配、提交、保全和新候选检查链路，不宣称线上模型质量或真实 GitHub 发布。通知测试覆盖实际子进程、数据库权限、并发领取与真实回环 HTTP；没有向真实 Bark 用户发送消息。完整源码绑定 Gate 结果另随开发记录保留。

## 仓库作用域

作用域由核心准入执行，以稳定的内部 `repository.id` 标识仓库；不使用 GitHub ID、目录、分支或长期工作区身份。同一仓库的多个 Run、保留工作区共用授权。作用域只有 `all` 和非空仓库 ID 列表；`all` 包含之后注册的仓库，但不代表自动选择所有执行插件。

管理员通过部署数据库的 `plugin_scope` 注册原生适配器与项目 Hook 的授权。固定身份为 `agent:codex`、`validation:native`、`delivery:github`，项目 Hook 为 `hook:<受审 name>`，通知为 `notification:<id>`。实现选择仍取自已有阶段合同、固定适配器和受审 Hook 配置，不能通过此表选择另一实现。通知登录及 Agent 均无此表权限。

```sql
-- 必须先登记作用域，再登记通知登录；没有隐式全局默认值。
INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled)
VALUES ('notification:A','all','{}',true),
       ('notification:B','repositories','{42}',true);
INSERT INTO notification_plugin(id,database_role,enabled)
VALUES ('A','notifier_a',true),('B','notifier_b',true);
-- 收窄/禁用会递增版本，保留历史；不会删除原事件或重置预算。
UPDATE plugin_scope SET kind='repositories',repository_ids='{42}'
WHERE plugin_id='agent:codex';
```

仓库列表拒绝空值、重复、非正数、不存在的 ID。迁移显式将旧通知、原生适配器和已受审 Hook 登记为 all；新插件必须显式登记。执行准入保存插件、调用、需求修订、仓库与作用域版本；开始、恢复和重试都复查当前授权，缺失或越界失败关闭。集成验证逐一检查所有输入仓库。收窄授权不会抹去已发生副作用；已有停止、保全和对账规则继续适用。

受控环境、验证扩展及交付 Hook 继续使用宿主受审 Registration，`scope_ref` 支持 `all`、`repositories:1,42` 和兼容的单仓库 `repository:42`。范围随原配置摘要冻结；实际调用按 `repository_revision` 校验所有选中扩展，宿主登记改变后原摘要失效，须通过已有显式恢复流程重新审核。作用域不能扩大资源根、凭据或操作权限。

生命周期事件在工作区建立前从需求/冻结修订取得仓库身份，写入事件快照。核心在创建 outbox 前筛选；领取、重试与手动重放检查当前授权及原作用域版本。插件收到 `repository_id` 和 `scope_version` 后仍自行选择通知内容和渠道。扩大范围不补投旧事件，也不改变已冻结的执行插件。收窄后保留投递历史与已用次数，越界事件不再返回给插件。

暂停使已通过的验证失效时，`resume` 不恢复旧证据。对于尚未发送的 GitHub 交付，显式 `revalidate_delivery` 可在原任务、候选和操作上重验：必须只有一个未释放交付，发布状态为 pending、attempts 为零、无任何发送尝试记录、PR 或合并操作。新验证成功后只更新原操作的当前验证引用，并在 `delivery_observation` 中追加 `validation_rebound`，记录旧／新验证及恢复授权事件；旧验证结果、失效标记和完整证据保留。未知发送或已有外部副作用仍须对账，不能通过此路径重置。

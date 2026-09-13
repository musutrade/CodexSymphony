# Personal AI Software Factory 综合方案

> 首版：评审通过的需求自动到 PR（Phase 0）、自动到合并与 Done（Phase 1）；长期演进为多仓库、多 Agent 软件工厂
> 版本：V9.0 Draft — 评审后零介入优先；各章节已按 0.1 前提同步（差异见 0.8）
> 日期：2026-09-07
> 更新：2026-09-13 — 按最少人工介入原则统一执行/验证、确定性产物、修复预算、环境自动恢复、资源清理与分期验收
> 状态：设计规格；实施范围和验收条件以第 22～24 章为准

---

## 0. 结论摘要

### 0.1 方案前提

本方案的设计前提只有一句话：

```text
人的判断集中在需求评审这一个点上；评审通过后，系统在正常路径上不再需要人介入。
```

由此推出两条贯穿全文的规则：

1. **人工介入是异常中断，不是流程步骤。** 每条 Requirement 默认走完全自动路径：
   评审通过 → 调度 → 编码 → 受控提交 → Gate → PR → CI → 合并 → 自动判定 Done。
   只有下列情况才进入待办箱等待人处理（输入修订同时按下述授权规则记录）：
   - Agent 判断需求有歧义或 AC 不可实现，主动停下提问；
   - 确实无法继续且需要扩大沙箱/网络授权的请求；单次拒绝不产生待办；
   - 安全类门禁失败（secret、架构违规、策略被撤销）；
   - 自动修复或阶段恢复预算耗尽、未知故障、无法自动恢复的环境/存储/交接问题；
   - 安全策略撤销、自动重验证失败，或合并后的验收失败；
   - 运维恢复/清理达到告警阈值、必要 reviewer 确认等待超时，以及 Policy 显式开启的人工步骤。
   用户主动修改 Contract/AC 是新的明确输入，保存时展示影响并记录重新授权；系统按分期
   保全工作、撤销旧动作并继续，不额外要求重复确认。已合并的需求转 follow-up Draft。
2. **需求评审的通过标准是"每条验收标准都可机器验证"。** `verification_method` 默认为
   `automated_test` / `ci_check` / `gate_check`；不能机器验证的 AC 必须改写，或由评审者显式
   标记 `waived` 并记录理由。不允许默认 `manual_review`。这是系统能自动判定 Done 的唯一来源，
   评审在这一步多花的时间，是从后面所有检查点里省出来的。

V8.1 中的三端审批、人工 Merge、手机逐项人工验收、用户选定 Review 意见等能力，降级为
"信任建立期"的**可选模式**：Repository Policy 可以打开它们，默认关闭；它们不再是任何 Phase 的
交付门槛。

已有授权内，平台先按阶段自动分类、探测、恢复和验证；只在机器无法判断、需要新授权或
预算耗尽时请求人工。环境 blocker 由平台有限探测，满足条件后自动恢复，不要求例行点击
Unblock。待定人工问题、安全撤销和权限扩大不能通过环境恢复绕过。每条待办包含已完成工作、
阻塞证据、已尝试动作、最小决策范围和决定后恢复步骤；同一原因聚合，不重复唤醒 Agent。

### 0.2 自动化等级

用三个等级描述系统对人的依赖程度，各 Phase 以达到某一等级为目标：

| 等级 | 含义 | 人介入的点 |
|---|---|---|
| L1 | 评审通过的 Requirement 自动跑到 PR 创建 | 评审、合并 |
| L2 | CI 绿 + Gate 过 + AC 全部 verified → 自动合并 → 自动 Done；CI 红自动修复 N 次 | 评审、异常 |
| L3 | 自然语言描述 → 系统生成 Contract 草稿 → 澄清 Agent 挑刺 → 人评审 → 自动 Ready | 评审、异常 |

L2 是本方案的核心目标；L1 是它的必经中间态；L3 降低评审本身的成本。

### 0.3 从 Symphony 继承什么

吸收 OpenAI Symphony 的执行内核，不把它"单进程、单 Tracker"的参考实现当作平台边界：

- Agent 永远在独立 workspace / worktree 中运行；
- 调度状态只由一个协调层修改；Worker 退出不等于 Requirement 完成；
- active run 定期向外部系统对账，restart 后基于外部状态恢复；
- Provider 凭证由主进程持有，不注入 Agent 环境；
- Agent 运行必须有超时、stall 检测和可观测事件；
- Workflow 配置错误不覆盖最后一份有效配置。

Symphony 的参考实现是 Elixir，本平台用 Rust 重写控制面。选择 Rust 的理由：长期常驻的
调度 + Supervisor + 子进程管理需要稳定的资源占用；方案中大量"必须原子 / 必须校验影响行数"
的不变量适合用类型系统守住；Codex 协议 schema codegen 与 serde 配合顺手；与配套仓库
技术栈一致。代价是 `app_server.ex` 等模块只能参考不能复用，需要重新踩一遍坑。

### 0.4 三个真相

```text
Requirement 状态是平台唯一业务真相
AgentRun 状态是执行事实
GitHub / CI / Review 是外部观察事实
```

外部事实可以触发 Requirement 状态转换，但不直接替代它。`Submitted` 只表示 PR 已交接；
`Done` 只能由最终 SHA 上的自动证据判定，不能由"PR 已合并"冒充。

### 0.5 完成声明、封存与交接

Agent 只在 workspace 内修改文件，通过受控工具 `create_local_commit` 请求本地提交，通过
`report_completion` 显式声明完成；不持有 GitHub 凭证，不 push，不创建 PR。平台持久化声明、
停止写入、在不可变 SHA 上运行受信 Gate、封存产出清单，再由独立的 HandoffOperation 通过
outbox 幂等执行 push / PR。交接重试不重新启动 Agent。

### 0.6 分期术语

```text
Phase 0a  本机闭环         L1，localhost，单进程，第一条真实需求跑到 PR
Phase 0b  远程可用         L1 + Cloudflare Access + 手机 Web + 一个通知渠道
Phase 0c  加固             L1 + 低权限 executor、受信 Gate、完整恢复对账、备份
Phase 1   自动闭环         L2，自动合并、自动 Done、自动 CI 修复
Phase 2   降低评审成本     L3 + 多仓并行
Phase 3   可选编排         Planner、子任务依赖、公平调度、Discussion
Phase 4   扩展             多 Runtime、模型 fallback、预算、Remote Worker、PWA

V1 = Phase 0a + 0b + 0c + Phase 1
```

Phase 0a 完成即开始日常使用；后续 Phase 不反向扩大 0a 的范围。实施范围以第 23 章为准，
第 24 章定义 V1 的交付门槛。

其余章节中出现的"MVP / Phase 0"统一指 Phase 0a～0c 合计；某项能力具体属于 0a、0b 还是 0c，
以第 23 章的清单为准。"人工 / 用户确认 / 审批"字样，除 0.1 列举的异常类型外，
均应理解为 Repository Policy 可选开启的信任建立期模式，不是默认路径。

### 0.7 首版路径

```text
写需求（描述 + 可机器验证的 AC）
    → 评审通过 = Ready
    → Scheduler 领取 → worktree → Agent 编码（沙箱内自动放行）
    → create_local_commit → hook Gate → report_completion
    → 平台验证 / 封存 → push → PR                       ← Phase 0a 终点（Submitted）
    → CI → 红则自动修复（预算内）→ 绿
    → 自动合并 → 最终 SHA 验证批次 → Done                 ← Phase 1 终点

异常 → 平台按阶段有限恢复；需要新授权/人工判断或恢复耗尽 → 一次待办 → 决策后自动续作
```

### 0.8 与 V8.1 的差异

本版（V9.0）相对 V8.1 的方向变化如下，各章节已按此同步：

| 变化 | 章节 |
|---|---|
| `verification_method` 默认 `automated_test`；`manual_review` 需 Policy 允许并附理由 | 7.1、7.2 |
| `POST /ready` = 评审通过，直接进队列；`/start` 降为别名 | 7.1、16.2 |
| 自动 Merge 默认开启，条件全部机械判定；`Merging` 从 Phase 1 启用 | 4.1、11.5、16.5、17.5 |
| Done 由 post_merge 自动证据判定；人工验收仅 Policy 开启时出现 | 4.1、7.2、7.6 |
| 修复预算默认 3、Policy 可配；RepairAttempt 统一覆盖本地/CI/Review，自动触发，不要求人选意见 | 12.5、12.6.4、13、14.2、28 |
| 声明后验证失败自动重跑一次带失败摘要的新 Run，不直接转人工 | 12.6.4、28 |
| 沙箱内自动放行、沙箱外默认拒绝（Policy 可改为 ask）；只有 Agent 提问必然进待办箱 | 8.1 |
| 输入漂移默认自动分类与重验证，失败才进待办箱 | 7.6 |
| 三端审批 / CLI 降为可选模式（0b P2） | 1.2、8.1、16.8、17.6、22 |
| 完整租约 / 恢复屏障 / digest / executor / Harness-Gate 推到 0c；0a 单进程简化 | 5.1、6.3、12.6、21.1 |
| 描述→Contract 草稿 + 澄清 Agent 提前到 Phase 2 | 7.3 |
| **网络授权改为需求内声明**（预设 / 域名 / 拒绝），白名单外由 Codex 本地代理直接拒绝，不产生审批请求 | 16.9、8.1、22.4、28 |
| 首页待办箱以"为空"为设计目标；新增介入率指标 | 18.4、19.3 |
| 新增风险 8（自动合并合入错误需求）、风险 9（需求质量成为瓶颈） | 26 |
| 第 23 章按 S / 0a / 0b / 0c / 1 / 2 / 3 / 4 重排并标 P0/P1/P2；第 24 章改为按 Phase 的 V1 DoD | 23、24 |
| 2026-09-13：预检/磁盘保护前移 0a；规则优先修复包，Phase 1 受信证据复用与重复失败停止 | 12.3、12.7、19.3、21.5、22～24、28 |
| 阻塞声明禁止自动续轮；恢复必须保全工作快照并注入上下文，有声明只恢复平台阶段 | 3.7、5.1、8.1、8.4、22～24、28 |
| 单仓异步资源回收前移 0a：持久化清理任务、引用保护、重试与补漏，Agent 不等待清理 | 10.6、14、19.3、21～24、28 |
| SoL-Pi 复评：0a 输出归档与分页、Phase 1 动作合并实验、Phase 2 压缩经济性实验；逐项对照 | 8.5、8.6、12.7.6、19.3、22～24 |
| 一致性修订：执行与验证分离、平台候选产物、RepairAttempt、环境自动解除、输入替代、清理告警聚合 | 3～4、8、10、12～14、22～24、28 |
| 阻塞体验补齐：诊断包、原因确认程度、六项可操作卡片、有限只读诊断与独立预算 | 3.8、8.7、14、16.7、18.4、19.3、22～24、28 |
| 合并事实与 fast-forward 双校验、分阶段 AC、脱敏视图收据、固定队列统计口径 | 7、11～14、17、19、23.S |

保留的信任建立期开关（Repository Policy，默认值即零介入路径）：

```text
auto_merge                    = true
gate_recovery_policy.mode     = auto      budget = 3
require_fix_confirmation      = false
allow_manual_review           = false
require_manual_start          = false
require_manual_revalidation   = false
sandbox_escape_handling       = deny      （可按类别改为 ask）
network_defaults              = []        （Repository 常见依赖域名与预设）
allow_requirement_network_scope = true    （允许需求级声明网络范围，见 16.9）
```

---

## 1. 产品定位

### 1.1 一句话定义

Personal AI Software Factory 是一个把"评审通过的需求"自动变成"已合并代码"的个人软件工厂：
人负责需求评审和异常处理，Agent 负责编码，平台负责调度、验证、合并和交接。
首版（Phase 0a～0c）先做到评审后自动到 PR，Phase 1 做到自动合并与自动判定 Done；
手机和电脑是查看进度、处理异常和写需求的入口，不是每条需求的必经关卡。

它把以下工作串成一个可恢复的系统：

```text
自然语言需求
    → 可选设计文档
    → Requirement Contract
    → 调度
    → 隔离执行
    → Git
    → PR
    → CI
    → Review
    → 修复
    → 完成
```

### 1.2 产品不是什么

V1 不做：

- Multi-Tenant SaaS
- 企业组织和 RBAC 体系
- 通用 BPM / Workflow Engine
- 通用 Issue Tracker 替代品
- 直接把 Agent 放到生产环境自动部署
- 同一 Requirement 跨多个 Repository 的分布式事务
- 一开始支持多个 Agent Runtime

“Single User”只表示领域模型不引入租户层，不表示 HTTP API 可以无认证暴露。

首版不做自建代码审查器、独立移动 App、完整 PWA/offline、多 Agent 协作、自动需求拆分或
多仓并发；这些不能成为日常使用的前置条件。平台内 Merge 从 Phase 1 起作为自动合并的实现，
不是首版范围。首版"编排"指排队、启动、异常暂停、继续、停止、重试和交接，不承诺 Planner/DAG 能力。
独立 Codex CLI/IDE 会话不属于平台的 AgentRun，不能自动共享租约、上下文或 PR 交接。

人工审批、人工 Merge、逐项人工验收和终端审批 CLI 是 Repository Policy 可开启的信任建立期模式，
不是默认路径，也不是任何 Phase 的交付门槛（见 0.1）。

### 1.3 GitHub 的角色

GitHub 负责：

- Repository
- Branch
- Commit
- Pull Request
- Review
- Checks
- GitHub Actions
- Webhook

平台负责：

- Project / Repository 注册
- Requirement
- Requirement 状态
- Agent 调度
- Agent 生命周期
- Workspace / Worktree
- Agent Run 和事件历史
- CI / Review 事实归档
- Recovery / ReviewFix
- 可选 Discussion 和仓库 Git 文档输入

GitHub Repository 不是 Requirement 的唯一数据源。平台自己的 Requirement 数据库才是调度真相。

### 1.4 首版使用边界

- 开发机常驻局域网，执行不依赖浏览器在线；手机切后台或关闭页面不取消任务。
- 正常路径零介入：评审通过后，编码、提交、Gate、PR（Phase 1 起含 CI 修复、合并、Done）
  全部自动完成。人只处理待办箱里的异常项；待办箱为空是系统的正常状态。
- 异常处理入口是电脑和手机浏览器；轻量平台 CLI 是可选增强（0b P2）。任一入口的决策写入
  同一 `runtime_request`，其余入口同步已处理结果。
- Phase 0a～0c（L1）由用户在 GitHub 合并；Phase 1 起默认自动合并，Repository Policy 可以关闭
  以进入"先看一眼再放开"的信任建立期。
- Agent 在沙箱和网络白名单内的操作自动放行；白名单外的请求默认拒绝并记录，不为此打断人。
- 第一条真实需求跑通后即可开始日常使用；不等待后续 Phase 的全部架构能力。
- 不为赶进度降低 worktree 隔离、凭证保护、受控提交、决策幂等或重启防重复执行要求；
  OS 级隔离在 0c 补齐，此前只接管自己可信的仓库。

---

## 2. 从 Symphony 继承什么

Symphony 的语言无关规格提供了非常好的执行内核：

- 单一调度权威
- active run reconciliation
- 全局并发控制
- retry / exponential backoff
- per-issue workspace
- workspace lifecycle hooks
- strict prompt rendering
- Codex app-server session
- provider-native tools
- token / rate-limit observability
- restart 后基于外部状态恢复

当前 Elixir 参考实现中的关键模块包括：

```text
Orchestrator
AgentRunner
Workspace
Codex.AppServer
Tracker
WorkflowStore
StatusDashboard
```

### 2.1 必须保留的原则

1. Agent 永远在独立 workspace/worktree 中运行。
2. 调度状态只能由一个协调层修改。
3. Worker 退出不等于 Requirement 完成。
4. 正常退出和异常退出使用不同的 retry 语义。
5. 每次 active run 都要定期向外部系统重新对账。
6. Provider 凭证由主进程持有，不注入 Agent 环境。
7. Workflow 配置错误不能覆盖最后一份有效配置。
8. Agent 运行必须有超时、stall 检测和可观测事件。

### 2.2 必须改变的地方

当前 Symphony 的配置和 Tracker 边界是单例式的：

```text
一个 WORKFLOW.md
一个 tracker.kind
一个 tracker.provider.repo / project_slug
一个 Config
一个 Tracker Adapter
一个 Orchestrator State
```

这对单项目参考实现足够，但不适合作为多项目平台。

新平台必须改成：

```text
Global Scheduler
    ├── ProjectRuntime A
    │     ├── RepositoryRuntime A1
    │     └── RepositoryRuntime A2
    ├── ProjectRuntime B
    │     └── RepositoryRuntime B1
    └── ProjectRuntime C
          ├── RepositoryRuntime C1
          └── RepositoryRuntime C2
```

所有运行时调用都必须接收明确的 `ProjectContext` / `RepositoryContext`，不能隐式读取一个全局 Tracker。

---

## 3. 核心领域模型

### 3.1 Project

Project 是个人开发者管理的一组相关软件资产和 Requirement 集合。

```text
Project
├── name
├── description
├── default_model_policy
├── knowledge_policy
├── max_concurrent_agents
├── workspace_root
├── enabled
└── repositories
```

Project 不是 GitHub Project V2 Board 的同义词。

如果未来接入 GitHub Projects V2，需要单独建立外部映射：

```text
Project
└── external_project_refs
      └── provider = github_projects_v2
```

### 3.2 Repository

Repository 是一个可被 Agent 独立检出和修改的代码仓库。

```text
Repository
├── project_id
├── provider = github
├── owner
├── name
├── default_branch
├── github_installation_id
├── workflow_path
├── knowledge_policy
├── gate_provider = none | harness-gate | custom
├── gate_config
├── gate_recovery_policy
├── trusted_gate_policy_revision_id
├── dispatch_epoch
├── canonical_clone_path
├── canonical_base_ref
├── max_concurrent_agents
├── test_commands
├── enabled
└── credential_ref
```

`knowledge_policy` 在 PostgreSQL 中以 JSONB 存储；下方 TOML 仅是配置形态示意，不是平台
的第二份配置真相。`canonical_clone_path` 是平台维护的 canonical bare clone 或工作树，
`canonical_base_ref` 是冻结输入时读取知识文件和 `WORKFLOW.md` 的默认 ref。

一个 Project 可以拥有多个 Repository：

```text
E-Commerce
├── frontend
├── backend
├── worker
└── infrastructure
```

### 3.3 Requirement

Requirement 是唯一进入调度队列的业务实体。

核心字段：

```text
id
project_id
repository_id
parent_id
root_requirement_id
type
title
description
priority
state
state_version
generation
requirement_source
recovery_source_type
recovery_source_id
current_revision_id
frozen_context_input_digest
last_validated_at
last_validated_commit_sha
revalidation_status
failure_code
failure_phase
retry_class
manual_action_required
target_pull_request_id
target_branch
retry_policy
model_policy_id
owner_id
lease_token
leased_until
heartbeat_at
not_before_at
created_at
updated_at
```

Requirement 类型：

```text
Feature
Bug
Refactoring
CiRecovery
ReviewFix
Manual
```

`requirement_source` 是一个通用来源值，而不是多套来源字段：

```text
requirement_source
├── source_type
└── source_ref
```

允许的 `source_type` 包括：

```text
direct
discussion_message
repository_document
external_issue
ci_failure
review_comment
none
```

`source_ref` 可以是 Discussion 消息 ID、仓库内相对路径 + commit SHA、外部事件 ID 或
空值。它只用于追溯，不参与 Requirement 的执行判定。

从 Phase 0 起每个 Requirement 必须绑定一个 Repository：

```text
Requirement → exactly one Repository
```

这不限制平台处理多个 Repository，只限制一次 Agent Run 的修改边界。

`acceptance_criteria` 不再作为 Requirement 的第二份字段；它只存在于
`RequirementContract.acceptance_criteria`，而 Contract 只存储在
`requirement_revisions.contract`。Requirement 表中的验收视图必须从当前
`requirement_revision` 派生，避免双写漂移。

Recovery / ReviewFix 的来源使用显式字段：

```text
recovery_source_type = ci_failure | review_batch | null
recovery_source_id
```

ReviewFix 使用平台不可变 review_batch ID 作为聚合来源，各意见版本在 review_fix_items 独立去重。
非 Recovery Requirement 的两个字段必须为 `null`。同一来源只允许创建一个对应的修复
Requirement（见 14.3）。

`root_requirement_id` 统一使用这一命名：根 Requirement 的值为自身 ID，Recovery /
ReviewFix 子 Requirement 指向根 Requirement。`total_recovery_attempts` 只在根
Requirement 上维护，从 Phase 1 的 RepairAttempt 预留记录计数，子任务读取根任务的聚合值。

根 Requirement 的 UUID 由应用在创建时先生成，并同时写入 `id` 与 `root_requirement_id`；
不依赖数据库触发器在插入后回填。

`state_version` 用于领域命令的乐观锁；`generation` 是执行授权代次。取消、Contract/AC
变更、重新授权执行时递增 generation，同时使旧审批、完成声明的交接授权和未发出的 outbox
命令失效。历史记录不删除。Repository 的 `dispatch_epoch` 单调递增，用于隔离旧执行者，
不得随 lease 行删除而重置。

### 3.4 AgentRun

AgentRun 表示一次实际执行尝试。

```text
AgentRun
├── id
├── requirement_id
├── project_id
├── repository_id
├── reason
├── attempt
├── generation
├── dispatch_epoch
├── runtime
├── provider
├── model
├── model_policy_snapshot
├── prompt_version
├── requirement_revision_id
├── context_input_digest
├── context_version
├── base_commit_sha
├── change_base_commit_sha
├── artifact_commit_sha
├── artifact_branch
├── completion_declaration_id
├── blocker_report_id
├── resumed_from_run_id
├── recovery_plan_id
├── repair_attempt_id
├── work_snapshot_id
├── artifact_manifest_id
├── handoff_operation_id
├── workspace_id
├── branch
├── target_pull_request_id
├── thread_id
├── turn_id
├── process_id
├── execution_group_id
├── process_started_at
├── worker_incarnation_id
├── approval_policy_snapshot
├── sandbox_policy_snapshot
├── gate_invocation_id
├── failure_receipt_id
├── failure_receipt_source_sha256
├── gate_result_digest
├── gate_report_path
├── status
├── started_at
├── finished_at
├── input_tokens
├── output_tokens
├── total_tokens
├── estimated_cost_microusd
└── error
```

AgentRun 必须保存执行时的配置快照，不能通过当前 Requirement 配置反推历史。
`push_status` / `pr_status` 从 HandoffOperation 派生，不在 AgentRun 中重复维护。

`context_input_digest` 只描述冻结的输入；`artifact_commit_sha` 描述经平台验证并封存的本地
commit。Agent 可以通过受控工具创建中间 commit，但中间 commit 不是已完成产出。
Agent 不负责 push、创建/更新 PR 或调用 GitHub API；这些外部副作用由平台交接流程执行。

普通重跑在旧执行者已隔离、交接已对账，且 PR 尚未创建或仍开放时，默认复用原 `ai/req-*` 分支；
`base_commit_sha` 是本次 Run 的起始 HEAD，`change_base_commit_sha` 是该需求变更相对的
集成基线。允许复用有明确来源的中间 commit，但仍须新的完成声明和验证，不要求为了重试而
制造无意义提交。已合并 PR 不复用，后续改动创建 follow-up Requirement。用户选择新分支时，
旧 PR 的关闭必须作为独立、显式授权的操作。

`thread_id` / `turn_id` / `process_id` / `worker_incarnation_id` 只用于审计和重启对账。
MVP 不恢复原 app-server 进程或 thread。先确认旧执行组已退出或被隔离，再决定恢复平台验证、
继续交接，还是创建新 AgentRun；不能仅凭 incarnation 不同就释放写入权。

职责边界固定为：

```text
Agent：
  在 workspace 内读取 Context、修改文件，通过 create_local_commit 请求本地提交；
  不持有 GitHub 凭证，不 push，不创建或更新 PR，不调用 GitHub API。

Platform / GitHub Adapter：
  持久化完成声明并停止 Agent 写入；
  确认执行完整性并保存 candidate，独立运行受信 Gate，封存不可变产出清单；
  通过 outbox 幂等执行 push；
  按 repository + branch 查找或创建 PR；
  只有 PR 已创建且关联已持久化，才允许 Requirement 进入 Submitted。
```

### 3.5 完成声明、封存产出与交接

以下记录各有独立职责，不能用“发现一个 commit”替代：

| 记录 | 必备绑定 | 意义 |
|---|---|---|
| CompletionDeclaration | run_id、generation、revision_id、context_input_digest、声明时 HEAD、summary、received_at | Agent 明确请求结束；先持久化再回复工具成功 |
| ArtifactCandidate | requirement/revision/generation、固定 SHA/tree、source_type/source_id、parent_candidate_id、context snapshot | 不可变待验证产物；来源为 agent_declaration 或 deterministic_repair |
| DeterministicRepair | repair_attempt_id、原 candidate、Policy 授权、工具/参数摘要、前后 diff、新 candidate | 平台受控修复执行事实，不是 Agent 完成声明 |
| ArtifactManifest | candidate_id、artifact SHA/tree、change_base SHA、snapshot_id、Gate policy/result、sealed_ref、sealed_at | 平台验证通过并保存在 Agent 不可写的位置，来源沿 candidate 追溯 |
| HandoffOperation | requirement_id、run_id、generation、manifest_id、verification_batch_id、input_authorization_event_id、repository/branch、expected_remote_sha、operation_attempt | 平台对固定产出的 push/PR 交接；重验证时保留显式授权引用 |

CompletionDeclaration、ArtifactCandidate 与 ArtifactManifest 不可变；每个 Run 最多一个被接受
的完成声明、每个声明最多一个 candidate、每个 candidate 最多一个 manifest。重复请求返回原记录，
参数不同的重复请求拒绝。Agent 修改 HEAD 或 Contract/AC 变化后继续编码需要新 Run，不得修改
已有声明的 SHA。平台确定性修复生成新的 candidate，引用 DeterministicRepair 和原 candidate，
无需新 AgentRun 或虚构声明；初始 candidate 必须源于有效 Agent 声明，修复链不能凭空授权。
其他输入变化按 7.6 自动/人工重验证规则处理，不回写历史输入。交接的 run_id 仅追溯最初执行；
实际发送对象以 manifest/candidate 为准，必须核对其当前授权和新验证证据。
Phase 0a/0b 的 context_input_digest 可空，统一以 input_identity_version=0、revision、generation、
已读取 WORKFLOW/策略快照摘要绑定；并非跳过身份检查。0c 起使用完整 MVP digest，旧记录不补写。

HandoffOperation 状态：

```text
Pending → Pushing → EnsuringPR → Succeeded
            └────────┴──────→ RetryWaiting → 原失败步骤
任一未完成阶段 → CancelRequested → Cancelled / Superseded
确定失败或重试耗尽 → Failed
```

`RetryWaiting` 不占 Agent 容量和执行 lease，但保留 branch reservation。网络调用结果未知时
必须先读远端对账，不能直接重发或宣称取消完成。交接消费者从 Phase 0 启用，其短周期对账不依赖
Phase 1 的全量 GitHubReconciler。详见 14.4。

### 3.6 External Facts

GitHub、CI 和 Review 事件是外部事实：

```text
PullRequest
CheckRun
WorkflowRun
ReviewComment
WebhookDelivery
```

它们可以触发 Requirement 状态转换，但不直接替代 Requirement 状态。

### 3.7 阻塞记录、工作快照与恢复计划（Phase 0a 起）

以下记录追加保存，独立于完成声明和验收证据，不新增业务主状态：

| 记录 | 必备绑定与内容 | 权威范围 |
|---|---|---|
| BlockerReport | requirement/run/generation/revision、请求幂等键、reason_code、证据引用、所需条件、Agent 自报进度、received_at | 执行受阻声明；根因和解除条件仍须平台核查 |
| WorkSnapshot | source_run_id、HEAD/base/tree、index 与工作树清单及内容摘要、遗漏项、captured_at、pending/complete/partial、存储引用 | 恢复中间工作；不是 ArtifactManifest 或验证通过 |
| RecoveryPlan | source_run_id、目标输入身份、blocker/snapshot/declaration/manifest 引用、恢复阶段、剩余步骤、解决证据、授权和预算引用 | 平台依据当前事实选择的恢复动作，不自动更新旧 Run |

新 Run 的 `resumed_from_run_id / recovery_plan_id / work_snapshot_id` 必须可追溯。
恢复事件和计划保留版本；后续解除阻塞追加解决记录，不覆盖原报告或累计消耗。
领取前预检 blocker 的 run_id 可空，绑定 requirement/输入身份，进度来源标记 platform；
运行时动态工具报告必须绑定当前 Run。BlockerReport 的平台分类附 recovery_mode=auto_probe|manual、probe_policy_id、probe_attempt、
next_probe_at、probe_deadline、resolution_event_id；Agent 不能指定 auto_probe 或解除自己的 blocker。

**工作快照的最低实现：** 在阻塞、停止或异常退出后，先禁止新写入并确认旧执行组静止，
再由平台保存 HEAD 及可达的中间 commit、暂存/未暂存改动、未跟踪的源码/测试/必要进度文件，
保留路径、增删、模式与二进制内容，能区分 index 和工作树。可重建缓存/构建输出按显式规则
排除并记录；不能因未跟踪或体积超限就静默丢弃实现。符号链接不跟随到工作区外，恢复时校验
路径 containment；不复制凭证、hooks 或任意 Git 配置。内容存入平台私有存储，确认可读及
摘要一致后再持久化完整快照引用；index 通过清单重建，不把原始 .git 目录当恢复接口。

快照保存失败、空间不足或内容不完整时标记 pending/partial 原因，保留原 worktree 与占用，
暂停清理和新写入者；不能声称已保存、覆盖原目录或从干净基线自动重跑。磁盘恢复后先重试
保存；唯一工作副本确实丢失时明确报告，只有人工明确选择从已知基线重新实施才可另建 Run。
恢复中间工作不要求使用已损坏的原路径：在隔离的新 worktree 按清单恢复并逐项验摘要，
保持该需求原有变更基线，不能隐式 reset/rebase 到最新 main。

平台生成 RecoveryPlan 的顺序固定为：取消/授权与旧组静止检查 → 未解决 blocker 检查 →
有效 manifest 则只交接 → 有 candidate 则独立验证/封存 → 有声明则确认执行完整性并生成 candidate → 无声明则预检、验证工作快照与恢复上下文，
之后才按预算创建新 Run。目标输入改变须按既有输入资格规则重新评估，不盲目套用旧快照。
重启只保全最后可读取的工作，不承诺恢复尚未写入磁盘的数据。尚未启动模型且确定没有
中间修改的准备重试可记录已验证的初始基线与空改动清单；不得把身份不明的目录当成空工作。


---

### 3.8 阻塞诊断与可操作待办

BlockerReport 记录“发生了什么”，DiagnosisReport 解释“哪些原因已确认”，ActionableBlockerView
回答“现在谁需要做什么”。后两者均关联原 blocker，不新增第二套业务状态或独立审批真相。未解决待办持有诊断包
与证据的受管引用；诊断完成不立即清理仍需用户查看或恢复使用的内容。

| 记录/视图 | 必备字段 | 约束 |
|---|---|---|
| DiagnosticBundle | blocker_id、revision/generation、失败阶段、输入/环境身份、工具退出状态、脱敏观察引用、固定探针结果、已尝试动作、工作保存状态、missing_evidence、created_at、digest | 平台机械组装并版本化保全；未采集到的信息显式列出，不要求用户重新找日志 |
| DiagnosisReport | blocker_id、bundle_id、producer=rules/model、cause_status=confirmed/suspected/unknown、原因说明、支持/反证引用、缺失信息、建议 action_id、created_at | 模型原因只能为 suspected/unknown；confirmed 必须有平台规则与有效探针依据 |
| DiagnosticAttempt | blocker_id、root_requirement_id、bundle_id、policy_snapshot、状态、预算预留、Runtime/模型/会话引用、累计 token/turn/耗时、报告引用 | 独立的只读诊断执行事实，非代码修复 Run；不改变 Requirement、Gate 或 GitHub 真相 |
| ActionableBlockerView | blocker_id/version、当前失败步骤、确认程度、已完成工作、系统已尝试/正在做的事、责任方、可执行操作、解除判据、恢复步骤、下次检查时间/需人工原因 | 从 blocker、报告、恢复计划与既有 runtime_requests 派生，不能由模型自由生成可执行按钮 |

每个操作包含固定 action_id、用户可读标签、对象与范围、执行主体（平台/用户/主机管理员）、
所需授权、参数 schema、预计影响、验证方式及 unavailable_reason。操作集合来自平台批准的
版本化 action catalog：重新检查环境、进入具体权限/需求修订入口、查看/导出脱敏诊断包、
重投已授权清理、取消任务等；不把模型输出的 shell、链接或“建议重试”直接变成执行入口。
引用外部设置页面时由适配器构造并验证目标；需要用户在宿主机操作时给出具体对象、步骤和
预期检查结果，命令仅来自批准的操作说明。未知原因不得伪造修复步骤来满足字段校验。

---

## 4. 统一状态真相

### 4.1 Requirement 状态

先按实施阶段阅读状态：

| 阶段 | 启用状态 | 成功终点 |
|---|---|---|
| Phase 0a / 0b | `Draft`、`Ready`、`Running`、`Submitted`、`Failed`、`Cancelled` | `Submitted` |
| Phase 0c | 增加 `NeedsRevalidation` | `Submitted` |
| Phase 1 | 增加 `Queued`、`WaitingCI`、`WaitingReview`、`Merging`、`Done`、`Blocked`、人工 `Unblock`、自动 Recovery / ReviewFix；自动 Merge 默认开启 | `Done` |
| Phase 2 | 扩展多仓调度与修复策略，复用现有状态 | 根任务恢复或 `Blocked` |

推荐状态：

```text
Draft
Ready
Queued
Running
Submitted
WaitingCI
WaitingReview
Merging
Done
Failed
Blocked
NeedsRevalidation
Cancelled
```

推荐转换：

```text
Draft
  → Ready
  → Queued              Phase 1+
  → Running
  → Submitted
  → WaitingCI
  → WaitingReview
  → Done
```

Phase 1 默认自动 Merge（Repository Policy 可关闭）。`WaitingReview` 在自动模式下的含义是
"等待 17.5 的合并条件全部成立"，包括 Policy 要求的 review thread 全部 resolved；默认 Policy 不要求
人工 approval。条件成立 → `Merging`（平台以 expected head SHA 调用 merge）→ 收到
`PullRequestMerged` 事实 → 启动最终验证批次；只有 7.6 的全部完成条件满足，才进入 `Done`。
合并后的自动验证保留 `WaitingCI`，通过 `completion_phase=post_merge_validation` 显示子阶段。
Policy 关闭自动 Merge 时，`WaitingReview` 等待用户在 GitHub 或平台内合并，其余流程相同。

异常转换：

```text
Running
  → Failed
  → Ready              执行阶段可重试故障，自动回队（not_before_at），不需要人

Failed
  → Ready              人工重试 Agent 执行，保留历史 Run
  → Running            人工重试仍有效的封存产出交接，不启动 Agent

Cancelled
  → Draft              人工重新打开

Running / Submitted / WaitingCI / WaitingReview
  → Blocked            Phase 1+，修复预算耗尽、安全类或第 28 章要求人工处理的阻塞
  → 原受阻阶段         auto_probe 条件满足或人工 Unblock 后，重新校验再继续；不盲目重新编码

Ready / Queued / Running / Submitted / WaitingCI / WaitingReview
  → NeedsRevalidation  Phase 0c+
  → Ready              Contract/AC 变了（显式 revise 保存视为重新评审），自动生成新 Revision 回队
  → 原阶段             Contract/AC 未变、产出在新输入下自动重验证通过，自动恢复
  → 待办箱             自动重验证失败，才需要人

任意未完成状态
  → Cancelled
```

`Done` 的定义必须明确：

```text
Phase 1 默认：
Requirement 的 PR 已合并，且最终 merged SHA 上的必需 CI、受信 Gate 和全部 AC 自动证据通过；
Policy 要求人工 Review 或 manual_review AC 时，这些证据也必须齐备。
```

`Submitted` 是平台的代码交接态：

```text
MVP / Phase 0：
candidate 来源链（Agent 声明或授权确定性修复）和封存产出有效，产出 commit 已 push，PR 已创建，平台已保存 PR 关联；
generation、当前输入资格、受信 Gate 与远端 head 校验全部通过
```

`Submitted` 不表示 CI、Review 或 Merge 已完成。Phase 0 即轮询并展示这些外部事实，
但不据此转换为尚未启用的内部状态。UI 分别显示“平台：已交接”“GitHub：开放/关闭/已合并”
和“CI：等待/运行/成功/失败/未知”，并显示最后同步时间。
Phase 1 通过 Reconciler（可选 Webhook 加速）驱动后续 `WaitingCI → WaitingReview → Done`。
收到 `PullRequestMerged` 事实前不能进入 `Done`；收到后仍必须满足 7.6 的最终验收。

重新执行规则：

```text
Failed → Ready：执行失败的人工 retry，默认复用仍开放的原分支和 PR；
Failed → Running：交接失败的人工 retry，只恢复有效 manifest 的交接操作；
Cancelled → Draft：人工重新打开，默认重新创建分支；
NeedsRevalidation：按 7.6 选择新执行或重新授权既有产出，不无条件回 Ready。
```

#### MVP 简化状态

Phase 0a / 0b 只使用核心 5 态：

```text
Draft → Ready → Running → Submitted / Failed
```

另保留 `Cancelled`（手动停止 / 取消）。`NeedsRevalidation` 从 Phase 0c 启用；此前
`Running` / `Submitted` 期间拒绝普通 PATCH 直接修改 Contract；使用受控输入替代命令，
一次显示影响并保存新 Revision，按下述规则处理，不要求先手工 Cancel 再点 Ready。


Phase 0a/0b 的受控输入替代（`POST /requirements/:id/revise`）先保存待应用 Revision 和用户授权，
原子撤销旧 generation、新工具/旧交接资格，异步停止旧组、保存完整 WorkSnapshot 并对账在途
交接；此期间显示 input_replacement_pending，不领取新 Run。全部完成后应用 Revision、冻结
新授权并回 Ready，系统核查旧工作适用性后生成 RecoveryPlan，不自动丢弃中间实现。普通表单
保存草稿不等于执行授权；仅显示影响并明确提交的 revise 命令视为重新评审。失败保留 pending
记录和现场，按自动恢复/人工异常规则处理。已合并则生成 follow-up Draft，不修改原完成事实。
0c+ 同一入口使用 NeedsRevalidation 状态承载，不增加第二套授权规则。

`Queued`、`WaitingCI`、`WaitingReview`、`Merging`、`Done`、`Blocked` 从 Phase 1 启用。
Phase 0 读取 PR/CI/基础 Review 摘要，仅作外部事实展示；不自动修复、不自动合并，
用户在 GitHub 合并。Phase 1 启用完整业务状态机、自动合并和最终验收。
`Submitted` 是 Phase 0 的终态交接，不是最终业务完成。`Running` 同时覆盖 Agent 执行和平台交接，
UI 从 active Run / HandoffOperation 派生 `execution_phase`，不得用"没有活跃进程"判定失败。

MVP 不暴露持久化 `Blocked` 状态。需要人工处理的失败统一为：

```text
Requirement.state = Failed
failure_code = ...
manual_action_required = true
```

Phase 1 启用独立的持久化 `Blocked` 状态。Blocked 表示不能继续，不等同于一定需要人；
8.1 的 auto_probe blocker 可自动解除，其他情况使用人工 Unblock。Phase 0 对应 Failed，
通过 manual_action_required 区分自动探测等待与人工异常，调度器不能仅凭 Failed 自动回队。

状态语义按分期固定：

| 阶段 | 成功终点 | 需要人工处理的失败 |
|---|---|---|
| MVP / Phase 0 | `Submitted` | `Failed + failure_code + manual_action_required=true` |
| Phase 1 | `Done`；有限修复成功后回到根任务流程 | `Blocked + failure_code + manual_action_required=true` |
| Phase 2 | `Done`；扩展修复成功后回到根任务流程 | 超过冻结修复上限进入 `Blocked` |

### 4.2 AgentRun 状态

```text
Created
  → PreparingWorkspace
  → StartingRuntime
  → Running
  → Finishing
  → Succeeded
```

异常状态：

```text
Failed
TimedOut
Stalled
Cancelled
```

AgentRun 在完成声明有效、写入静止、声明 HEAD 一致且工作树干净后进入 `Succeeded`，
表示 Agent 执行完成，不表示质量验证通过或交付完成。平台先保全固定 candidate，再独立
运行 VerificationBatch；Gate/AC 失败仅记录验证事实和编排决定，不将已成功的 Run 改成 Failed。
执行异常、未完成声明、工作区完整性异常仍按执行事实记录失败。
AgentRun 状态不能直接决定 Requirement 状态。例如：

```text
AgentRun = Succeeded
HandoffOperation = RetryWaiting
Requirement = Running
```

终态 AgentRun 不重开。对既有有效 candidate 的固定 SHA 重新验证时创建新 VerificationBatch，
保留原 Run 的执行历史；验证通过可以封存或重新授权产出并交接，不改写历史 Run。
Finishing 只覆盖停止旧组、完整性确认与候选保全，不含独立验证；UI 从 VerificationBatch
显示 validation 子阶段。验证占独立主机额度，不占 Agent slot。

MVP / Phase 0 的 AgentRun 不使用 `Blocked` 状态；运行层只记录 `Failed`、
`TimedOut`、`Stalled` 或 `Cancelled` 与对应 `failure_code`。持久化业务 `Blocked` 从
Phase 1 启用。

### 4.3 外部事实状态

外部事实不参与内部状态命名：

```text
GitHub PR: OPEN / CLOSED / MERGED
Check: QUEUED / IN_PROGRESS / SUCCESS / FAILURE
Review: COMMENTED / APPROVED / CHANGES_REQUESTED
```

适配层把这些事实转换成领域事件：

```text
PullRequestOpened
PullRequestMerged
CiPassed
CiFailed
ReviewReceived
ReviewApproved
```

---

## 5. 总体架构

```text
┌─────────────────────────────────────────────┐
│ Responsive Angular Web（PWA 后续增强）      │
│ Requirement / Project / Agent / PR / CI     │
└──────────────────────┬──────────────────────┘
                       │ HTTP 轮询（SSE 后续可选）
                       ▼
┌─────────────────────────────────────────────┐
│ Axum API                                    │
│ Auth / CRUD / Commands / Webhook            │
└──────────────┬──────────────────┬───────────┘
               │                  │
               ▼                  ▼
        PostgreSQL             Event Bus
        Source of Truth        Outbox / Inbox
               │                  │
               └────────┬─────────┘
                        ▼
                 Global Scheduler
                        │
       ┌────────────────┼────────────────┐
       ▼                ▼                ▼
 ProjectRuntime A  ProjectRuntime B  ProjectRuntime C
       │                │                │
       ▼                ▼                ▼
 Repository queues / capacity / policy
       │
       ▼
 Agent Supervisor + 多 Reconciler 组件
       │
       ▼
 Agent Runtime
       │
       ├── Codex App Server
       └── Future Runtime
       │
       ▼
 Workspace + Git Worktree
       │
       ▼
 GateRunner → Harness-Gate
        │
        ├── secret scan
        ├── architecture audit
        ├── project tests / lint / build
        └── machine-readable evidence
        │
        ▼
 GitHub App / PR / Checks / Review
       │
       ├── Success → state transition
       └── Failure → Recovery / ReviewFix Requirement
```

上图是目标形态；访问入口另见第 21 章的 Cloudflare Tunnel + Access。MVP 只启用单 Project /
单 Repository / 单 Agent，运行 Process、Workspace 和只读 GitHub 状态轮询组件，
加一个消费数据库 outbox 的通知发送器；它们均可在一个控制面进程内实现。
MVP 的输入漂移检查由 Scheduler / Supervisor 在关键时机同步执行，不依赖独立 DigestReconciler。

### 5.1 组件职责

#### API

- 鉴权
- Project / Repository / Requirement CRUD
- 启动、停止、重试、取消命令
- Discussion 和文档接口
- 待办聚合、权限审批和人工输入
- PR/CI/Review 摘要与同步时间
- GitHub webhook 接收和 SSE 订阅（Phase 1 可选增强）

#### Global Scheduler

- 扫描所有 `Ready` / `Queued` Requirement
- 检查依赖
- 检查全局容量（Phase 0 / Phase 1 固定 1；Phase 2 默认 1，可配置到 4）与同 Repository 串行约束
- FIFO + priority 排序（Phase 0 / Phase 1）
- 创建 AgentRun
- 维护 claim lease

Phase 0 / Phase 1 不实现 Project 级容量和 weighted round-robin 公平调度（推迟到 Phase 3）。

#### Supervisor

- 启动 Agent 子进程
- 监控退出
- 处理 timeout / stall
- 维护受控执行组、续租看门狗与退出确认
- 接受并持久化完成声明，停止写入后封存产出
- 把结果写入数据库事件

#### Reconciler 组件

拆分为五个独立组件，各自独立间隔、独立错误处理，通过 inbox / outbox 事件总线协调：

```text
ProcessReconciler       每 30 秒   执行组、完成声明与交接对账；按持久化计划有限探测并解除环境 blocker
WorkspaceReconciler     每 5 分钟  对账孤儿 workspace / worktree，向清理队列提交受管资源候选
GitHubReconciler        默认 60 秒 同步已关联 PR / CI / Review 摘要；Phase 0 只读，Phase 1 驱动闭环
DigestReconciler        事件触发   处理 InputSnapshotChanged，重算 context digest
ResourceCleaner        队列触发 + 每 5 分钟补漏  异步回收受管资源；Phase 2 扩展历史数据 GC
```

Phase 0a 只实现最小 `ProcessReconciler`：进程重启后对 Run 和独立平台任务统一对账，按
"manifest → 交接；candidate → 独立验证；声明 → 确认完整性并保全 candidate；
无以上记录 → 核查 blocker、保存并校验工作快照和恢复上下文，条件满足才按预算新建 Run"
处理（3.7、8.4）。已成功 Run 的验证仍可在途，不能因 Run 终态而跳过该平台任务。持久化阻塞优先于通用进程失联重试；
不得仅因 Issue active、进程退出或 attempt 仍有余额重新启动编码。
下面的 9 条冷启动规则、恢复屏障、execution_group 与 incarnation 核对从 Phase 0c 起完整实现。
Phase 0a 已有最小 `GitHubReconciler`，Phase 0b 增加 `WorkspaceReconciler`；
后者在 Phase 0 不创建修复任务或将 Requirement 标记 Done，轮询范围和限流规则见 11.5。
`NeedsRevalidation` 的最小输入 digest 检查从 0c 起由 Scheduler / Supervisor 在 `Ready`、Agent start、
提交/终态判定前执行。0a 的 ResourceCleaner 先实现 10.6 的单仓登记、延迟任务、重试与补漏；
完整 DigestReconciler、多仓资源协调和历史数据 GC 随 Phase 2 增加；
基本磁盘告警与低磁盘暂停从 Phase 0a 提供；完整备份和人工清理入口从 Phase 0c 提供（21.5）。

ProcessReconciler 的冷启动规则：

```text
1. 先为旧 Run、验证任务和 HandoffOperation 建立恢复屏障，暂不向对应 branch 发放新写入权；
   先检查取消和 generation 失效，失效者只允许停止/归档/对账，禁止进入后续执行恢复路径。
2. 按 execution_group_id、进程启动时间和 worker_incarnation_id 核对真实执行组；
   incarnation 不同只说明失联，不能说明已停止。终止整个执行组并确认所有子进程退出。
3. 无法确认退出或隔离成功时，保留恢复屏障和容量占用，记录人工处理原因；不得过期接管。
4. 没有持久化 CompletionDeclaration：保全中间 commit 和未提交工作快照，不 push、不补建 PR；
   若失联时尚有待回答的审批/人工输入或未确认送达的决策，先使旧请求失效并转人工，
   不自动重试绕过待定决策。存在未解除 BlockerReport 时禁止自动续轮或创建新 Run。
   其余旧 Run 结束后，按 3.7/8.4 核查预检、快照、恢复上下文与执行预算，再创建新 Run；
   新线程从已保存工作和剩余步骤继续，不能默认回到干净基线。
5. 有声明或 candidate 但没有 manifest：校验来源链、generation/revision/阶段身份，
   有 candidate 则恢复独立验证；仅有声明则先确认执行完整性并保全 candidate。确定性候选按其
   原候选与 Policy 授权恢复，不能要求新的 Agent 声明；
   HEAD 不同、声明失效或 Gate 不通过时拒绝交接，不以现存 PR 替代这些检查。
6. 有有效 manifest 或未完成 HandoffOperation：只恢复交接步骤，不创建新的编码 Run；
   已 push 未建 PR、PR 已建但响应丢失分别读远端事实后补齐。
7. generation 已取消或被新输入替代：禁止新的外部调用；已发出但结果未知的调用先归档对账，
   不推进 Submitted，不自动删除分支或关闭 PR。
8. Running 且没有 active Run、待封存声明、未完成验证批次或未完成交接，才是孤立状态；
   核实执行组已静止且无外部调用在途后，按原失败阶段和重试预算恢复或转人工。
9. Submitted 必须满足有效 candidate 来源链、manifest、当前授权/输入资格、Gate 和持久化 PR 关联；
   同一 branch 的恢复屏障只能在以上对账完成后释放。
```

MVP 不恢复原 app-server 的 thread/session；`thread_id` 和 `turn_id` 仅保留历史审计。

#### Agent Runtime

- Codex app-server 协议
- Thread / Turn 生命周期
- 事件解析
- 审批策略
- 动态工具
- Token 和 rate-limit 提取

#### GateRunner

- 对本项目仓库强制使用显式 `--project-root` 和 `--config` 调用 Harness-Gate。
- 配置和强制验证入口来自平台批准的不可变 policy snapshot，不从 Agent 的修改中自助更新。
- 本项目在提交前运行快速 `hook` profile，在 PR/CI 前运行全量 `verify --all` 或
  `verify --profile ci --all`。
- 对外部 Repository 只有在其 `Repository Policy` 显式声明 `gate_provider = harness-gate`
  时才启用 GateRunner；否则使用该仓库声明的其他确定性门禁或只归档外部 CI 结果。
- 解析 JSON/Markdown machine result，并把 invocation、configuration digest、scope、步骤状态、
  failure code 和报告路径写入 AgentRun。
- 门禁配置、报告、日志和临时服务的生命周期不由 Agent 自己决定，由平台 GateRunner 统一管理。

#### Handoff Worker

- 消费已封存产出的 outbox；不运行模型。
- 取得与 Agent 写入互斥的 Repository lease，核对 generation、branch reservation 和远端预期 SHA。
- 分别执行 push、PR 查找/创建，持久化每一步的调用意图、结果和重试时间。
- 处理取消与不确定网络结果；交接失败不降级为新的 AgentRun。

#### GitHub Adapter

- Repository / branch / commit / PR
- Checks / Actions
- Reviews
- Webhook
- GitHub App 认证

#### Notification Dispatcher

- 从 Phase 0 启用，仅实现并配置一个外部通知渠道，不建设多渠道路由系统。
- 消费数据库 outbox，发送待审批、待回答、执行失败和 PR 待处理的脱敏提醒。
- 通知只带受 Access 保护的平台链接，不携带批准令牌，不支持从通知直接执行危险操作。
- 通知失败独立重试，不阻塞 Agent 事件落库，也不产生新的编码 Run；详见 19.5。

---

## 6. 调度与容量（Phase 0 / Phase 1 简化）

Phase 0 / Phase 1 固定单 Project、单 Repository、单 Agent 执行。Phase 2 才开放多仓并行；
同仓串行约束继续保留。多 Project 公平容量管理推迟到 Phase 3
（见第 23 章）。

### 6.1 全局容量与 Repository 串行

调度一个 Requirement 必须同时满足：

```text
global_capacity_available（Phase 0 / Phase 1 固定 1；Phase 2 默认 1、可配置上限 4）
AND repository_serial_available（同一 Repository 同时最多 1 个执行/交接写入者）
AND runtime_capacity_available
AND dependency_satisfied
AND no_active_conflicting_run
AND no_unresolved_execution_group
AND no_pending_handoff_on_target_branch
AND network_policy_resource_available（16.9 的主机级白名单身份/占用匹配）
```

Phase 0 / Phase 1 不实现 Project 级容量，也不做 weighted round-robin 公平调度。

未启用诊断 Agent 时模型容量只统计 active AgentRun；交接重试等待不占该容量。
8.7 启用诊断 Agent 后，全局模型容量统计 active AgentRun + Queued/Running 中已预留容量的
DiagnosticAttempt；诊断与编码使用同一事务级调度锁，不能绕过全局并发 1。仅等待队列且未
预留容量的诊断不占额度，恢复先对账孤立的诊断执行组再释放。Handoff Worker 与 Agent 共用
Repository 排他写入权；独立验证任务使用隔离检出和单独的主机资源额度，不增加 AgentRun。
`dependency_satisfied` 的分期规则见 7.2：Phase 0 没有执行性依赖，不能把 Submitted 当作依赖完成。

### 6.2 调度策略

简单 FIFO 队列：

```text
priority DESC
    → created_at ASC
```

Repair Requirement（CiRecovery / ReviewFix）默认获得优先级加成，但仍受全局容量和同
Repository 串行约束，不能无限抢占普通 Feature。

### 6.3 Claim Lease

分期口径：

```text
Phase 0a / 0b  单进程，全局并发 1。只使用 requirements 上的 owner_id / lease_token /
               leased_until / heartbeat_at，加 agent_runs 的活跃部分唯一索引（14.3）。
               不实现 Repository dispatch lease、dispatch_epoch 递增、worktree_leases 与恢复屏障；
               字段和表保留，值固定为 0 / 空。仍必须有启动恢复闸门：单实例控制权、持久化
               Run/进程组标识与启动时间、generation/当前 incarnation 守卫；确认旧组静止、
               快照可恢复及交接对账前不放行新写入者。不能把未实现完整屏障理解为直接重跑。
Phase 0c+      启用下文的完整领取事务、双租约续租、epoch 隔离与 branch reservation。
```

每个待运行 Requirement 需要数据库 lease：

```text
requirement_id
owner_id
lease_token
leased_until
heartbeat_at
```

启动 Agent 前必须原子地取得 lease。MVP 没有 `Queued` 态，`Ready` 即可领取；Phase 1+
可在领取前先经过 `Queued`。

Phase 0 的 Repository lease 持有者可以是 Run 或 HandoffOperation；
Phase 1 增加 DeterministicRepair（holder_kind=deterministic_repair）和 MergeOperation（`holder_kind=merge`，自动或人工触发），同样核对 generation/dispatch_epoch，
恢复时先对账未完成合并结果，禁止结果未知时向该分支发放新写入权。
branch reservation 使用独立的
`worktree_leases` 记录，其寿命覆盖执行、封存和交接；它不会随短租约释放或超时而自动消失。
所有 claim 路径统一使用事务级 advisory lock，避免“容量检查”和“lease 占用”之间出现竞态。
active 数量从 `agent_runs` 及启用后的诊断容量预留事实派生，不维护会因 crash 泄漏的 `active_count`：

下面是 Agent 执行领取示例。封存恢复和 Handoff Worker 使用同一 Repository epoch/租约机制，
但不插入新 AgentRun，也不执行 Ready → Running 领取；交接只校验原授权仍有效。

```sql
BEGIN;

SELECT pg_advisory_xact_lock(hashtextextended('factory:global-scheduler', 0));
SELECT pg_advisory_xact_lock(hashtextextended('factory:repository:' || $repository_id, 0));

SELECT dispatch_epoch
FROM repositories WHERE id = $repository_id FOR UPDATE;
-- 在该锁内核对旧执行组已静止、无恢复屏障、目标 branch 无未完成交接，
-- 依赖已满足且 Requirement 属于该 Repository；否则 ROLLBACK。

SELECT count(*) AS active_count
FROM agent_runs
WHERE status IN ('created', 'preparing_workspace', 'starting_runtime', 'running', 'finishing');
-- 应用必须实际比较 active_count < $global_limit（Phase 0 / Phase 1 固定 1）；
-- 不满足时 ROLLBACK，并记录 scheduler_capacity_unavailable。
-- 启用 8.7 后，在同一锁内加上已预留容量的 diagnostic_attempts；该示例只展示编码 Run 查询。
-- 等价伪代码：IF active_count >= $global_limit THEN RAISE capacity_unavailable;

UPDATE repositories
SET dispatch_epoch = dispatch_epoch + 1
WHERE id = $repository_id
RETURNING dispatch_epoch;
-- 返回值绑定为 $epoch；只有整个事务成功，该代次才生效。

INSERT INTO repository_dispatch_leases(
    repository_id, owner_id, lease_token, dispatch_epoch, holder_kind, holder_id,
    leased_until, heartbeat_at
)
VALUES (
    $repository_id, $owner_id, $token, $epoch, 'run', $run_id,
    clock_timestamp() + interval '60 seconds', clock_timestamp()
)
ON CONFLICT (repository_id) DO UPDATE
SET owner_id = EXCLUDED.owner_id,
    lease_token = EXCLUDED.lease_token,
    dispatch_epoch = EXCLUDED.dispatch_epoch,
    holder_kind = EXCLUDED.holder_kind,
    holder_id = EXCLUDED.holder_id,
    leased_until = EXCLUDED.leased_until,
    heartbeat_at = EXCLUDED.heartbeat_at
WHERE repository_dispatch_leases.leased_until < clock_timestamp();
-- 应用必须检查影响行数 = 1；已存在且未过期时 ROLLBACK。

UPDATE requirements
SET state = 'running',
    state_version = state_version + 1,
    owner_id = $owner_id,
    lease_token = $token,
    leased_until = clock_timestamp() + interval '60 seconds',
    heartbeat_at = clock_timestamp(),
    not_before_at = NULL
WHERE id = $id
  AND state IN ('ready', 'queued') -- Phase 0 只允许 ready
  AND repository_id = $repository_id
  AND generation = $generation
  AND state_version = $expected_state_version
  AND (not_before_at IS NULL OR not_before_at <= clock_timestamp())
  AND (leased_until IS NULL OR leased_until < clock_timestamp());
-- 应用必须检查影响行数 = 1；否则 ROLLBACK。

INSERT INTO agent_runs(
    id, requirement_id, repository_id, attempt, status, generation,
    dispatch_epoch, worker_incarnation_id
)
VALUES (
    $run_id, $id, $repository_id, $attempt, 'created', $generation,
    $epoch, $worker_incarnation_id
);
-- UNIQUE(requirement_id, attempt) 冲突时 ROLLBACK。
-- 同事务取得/转交该 Requirement 的 branch reservation，并写入事件。

COMMIT;
```

任一检查失败都必须回滚整个事务；即使 Repository lease 已先成功接管，只要后续
Requirement 更新或 AgentRun 插入失败，也不得留下短暂占用。事务提交前不得启动 Agent。
执行期间 Requirement lease 和 Repository lease 每 20 秒在同一事务续租，必须匹配
`owner_id + lease_token`、当前 generation/Repository dispatch_epoch，且两份 lease 均未到期。
禁止通过迟到心跳复活已过期租约。数据库不可达、任一更新影响行数不为 1 或到达续租安全期限时，
独立看门狗停止新工具请求并终止完整执行组；确认静止前不释放 branch reservation。
已接受完成声明后的封存恢复只能由重新取得当前 epoch 的平台验证者执行。

释放操作必须同时匹配 owner 和 token，避免旧 owner 误删新 owner 的 lease：

```sql
BEGIN;

UPDATE requirements
SET owner_id = NULL,
    lease_token = NULL,
    leased_until = NULL,
    heartbeat_at = NULL
WHERE id = $requirement_id
  AND owner_id = $owner_id
  AND lease_token = $lease_token;

DELETE FROM repository_dispatch_leases
WHERE repository_id = $repository_id
  AND owner_id = $owner_id
  AND lease_token = $lease_token;

COMMIT;
```

两条语句的影响行数都必须记录；Requirement lease 释放失败时不能静默认为 Run 已完成，
应进入对账队列。

释放前必须确认该持有者的本地执行组已静止；若有外部请求在途，还必须先解决结果不确定性。
数据库事件、工具请求、完成声明和封存写入都核对当前 generation 与 dispatch_epoch。
旧 epoch 的消息只能作为迟到审计事件保存，不得改变状态或触发外部写入。

执行阶段可重试故障：终止并确认执行组退出，结束 AgentRun，释放两类短租约，Requirement
回到 `Ready` 并设置 `not_before_at`，到时创建新的 Run。已存在声明/manifest 时优先恢复
平台验证或交接，不能套用重新编码路径。

交接阶段可重试故障：HandoffOperation 进入 `RetryWaiting`，Requirement 保持 `Running`，
释放短租约但保留 branch reservation，到时只重试交接步骤。等待期间没有 active AgentRun，
不得被 ProcessReconciler 当作孤立 Running；所有重试预算按第 28 章分别计算。

Lease 生命周期：

| 操作 | Requirement lease | Repository lease |
|---|---|---|
| 取得 | 执行/验证/交接持有者写入 owner/token/expiry | 旧写入者静止后取得新 epoch |
| 续租 | 每 20 秒，匹配 owner/token/generation 且未过期 | 同事务匹配 owner/token/epoch 且未过期 |
| 释放 | 持有者静止后清空匹配租约 | 删除匹配 owner/token 的行，epoch 保留在 repositories |
| 过期接管 | 到期且恢复屏障解除 | 到期、旧执行组静止且无不确定外部调用 |
| 执行重试等待 | 释放，回 `Ready`，设置 `not_before_at` | 释放；branch reservation 按恢复状态保留 |
| 交接重试等待 | 释放，保留 `Running` 的交接子阶段 | 释放；目标 branch reservation 继续保留 |

### 6.4 冲突规则

以下情况必须互斥：

- 同一个 Requirement 的多个 AgentRun
- 同一个 PR 分支的多个写入型 AgentRun
- 同一个 workspace 的多个写入型 AgentRun

同 Repository 串行从 Phase 0 起覆盖 Agent、LocalGitBroker 和 Handoff Worker；
LocalGitBroker 使用当前 Run 的租约，不另发并行写入权。等待重试的交接也保留 branch reservation，
因此“没有 active AgentRun”不足以允许同分支再次执行。workspace 级冲突检测保留。

---

## 7. Requirement 录入和执行契约

### 7.1 Phase 0 / Phase 1 直接录入

Phase 0a 提供浏览器直接入口，0b 覆盖手机。单 Project / Repository 自动选中，
类型默认为 Feature、优先级默认为普通；非必要配置收进高级选项：

```text
选择 Project
    ↓
选择 Repository
    ↓
填写标题
    ↓
填写描述
    ↓
填写 Acceptance Criteria
    ↓
选择优先级 / 类型
    ↓
保存为 Draft
    ↓
评审通过 = Ready（唯一的人工决策点）
```

最小 Requirement 表单：

```text
Title
Description
Acceptance Criteria（逐条填写，至少一条；每条必须带可机器执行的验证方式和引用）
高级选项：Project / Repository / Type / Priority / Related Requirements / Model Policy
```

用户不直接编辑 Contract JSON。API 由表单生成 Contract，AC ID 自动生成。
`verification_method` 默认 `automated_test`，表单默认两个验证阶段；仅在高级选项中调整
阶段并说明理由。表单要求填写测试命令或选择已有
ValidationStep；`ci_check` 需选择 Policy 中声明的 Check 名称；`gate_check` 需选择受信 Gate 规则。
`manual_review` 必须显式选择并填写理由，且只有 Repository Policy 开启 `allow_manual_review`
时才可选；默认 Policy 不开启。评审通过（`POST /ready`）时校验：每条 AC 的 `verification_ref`
可解析、至少一条 `must` 级 AC 可机器验证；不满足则拒绝进入 `Ready`，并指出是哪条 AC。
不能伪造验证成功；未接入的验证源保持 pending。Phase 0 还要求至少一条可在本地 pre_merge 执行的自动 AC，
防止只配置 ci_check 而在首版没有实际本地验收。
`problem` 来自描述、`goal` 默认来自标题，范围和验证计划按模板生成并在评审页可见、可修改。
简单表单不降低 7.2 的 schema/AC 校验。描述→Contract 草稿生成与澄清 Agent 从 Phase 2 启用（7.3）。

评审通过即进入调度队列，没有独立的"确认执行"步骤；`Ready` 表示可以被 Scheduler 自动领取。
`POST /ready` 是评审通过动作：校验 Contract、冻结 revision、进入 `Ready`；对同版本重复调用幂等。
`POST /start` 保留为 `/ready` 的别名并额外唤醒一次调度扫描，不绕过 lease、容量、依赖或状态机检查。
Repository Policy 开启 `require_manual_start` 时（信任建立期），`/ready` 只进入 `Ready` 但不领取，
需另调 `/start`；默认关闭。

### 7.2 Requirement Contract

`Requirement Contract` 是平台唯一的可执行和可验收输入。它不依赖 ADR、OpenSpec 或
Discussion；这些内容如果存在，只能作为编写 Contract 时的辅助输入。

Contract 是结构化数据，只存储在 `requirement_revisions.contract` JSONB
中；`requirements.current_revision_id` 指向当前 Revision。`requirements` 表不保存
Contract 的第二份可写副本：

```typescript
interface RequirementContract {
  version: "1.0";
  problem: string;              // 自然语言描述
  goal: string;                 // 自然语言目标
  in_scope: string[];           // 每项一句话
  out_of_scope: string[];
  acceptance_criteria: AcceptanceCriterion[];
  validation_plan: ValidationStep[];
  constraints: string[];
  dependencies: DependencyRef[];
  network_access?: { presets: string[]; domains: string[]; denied: string[] };
  metadata: { created_at: string; created_by: string };
}

interface AcceptanceCriterion {
  id: string;          // AC-001 格式，平台自动生成
  description: string; // 必须可验证
  verification_method: "automated_test" | "ci_check" | "gate_check" | "manual_review";
                       // 默认 automated_test；manual_review 需 Policy 允许并附理由
  verification_ref: string; // 指向 ValidationStep、受信 Check selector、Gate 规则或人工验收规则
  verification_stages: ("pre_merge" | "post_merge")[]; // 默认两阶段；只在合并后可验证须评审明确批准
  stage_scope_reason?: string; // 非默认双阶段时必填，不能豁免 Policy 强制检查
  evidence_required: boolean;
  priority: "must" | "should" | "could";
}

interface ValidationStep {
  id: string;
  step: string;
  command?: string;
  expected_outcome: string;
}

interface DependencyRef {
  requirement_id: string;
  relationship: "blocks" | "depends_on" | "related_to";
}
```

Revision 存储约束：

```sql
requirement_revisions.contract JSONB NOT NULL,
requirement_revisions.contract_version VARCHAR(10) NOT NULL DEFAULT '1.0',

CHECK (contract ? 'version'),
CHECK (contract ? 'problem'),
CHECK (contract ? 'goal'),
CHECK (contract ? 'acceptance_criteria')
```

以上 CHECK 只是数据库底线；API 还必须按版本化 JSON Schema 验证字段类型、非空 AC、ID
唯一性、verification_ref 的可解析性和当前阶段支持的依赖类型。Contract 命令属于不可信输入，
只能在隔离验证环境执行，不能替代平台强制 Gate 或修改其权限。

验证流程：

```text
1. Contract 创建时，平台自动生成 AC ID（AC-001、AC-002…）
2. Agent 声明完成后，平台在封存前对固定 SHA 执行 pre_merge 适用的 automated_test / gate_check，
   写入 verified 或 failed；任一该阶段适用且未明确豁免的 AC failed 视为验证失败（见 12.6.4 的处理路径）
3. ci_check 在 Phase 0 只展示、保持 pending；Phase 1 起按精确 SHA 的 Check 结果自动写入。
   manual_review（仅 Policy 允许时存在）保持 pending 直到有人在待办箱处理
4. 每次验证追加到 requirement_acceptance_checks，绑定验证批次和目标 commit，不覆盖旧证据
5. Phase 0：有效完成声明 + 封存产出（含该阶段适用 AC 本地验证通过）+ 平台交接成功 → Submitted
6. Phase 1：PR 合并后建立最终验证批次，各阶段适用的所有 AC verified 或 waived，且最终必需 CI/Gate 通过 → Done；
   全程无需人参与，除非出现 manual_review AC 或验证失败
```

验收统一判据：priority 只表示业务优先级，不隐含跳过检查。默认每条 AC 在 pre_merge 和
post_merge 都适用；评审时可显式声明阶段并说明理由，仅合并后可验证的条件不阻塞合并前。
平台先验证阶段声明与 Policy 兼容，至少一条 pre_merge 自动 AC；不能把强制检查全部挪到合并后。
Phase 0 的 ci_check 仍 pending，只要求该阶段可执行的本地 AC 通过才 Submitted；这不是最终验收。
Phase 1 合并前要求所有 pre_merge 适用 AC 通过/明确豁免，Done 要求全部适用阶段证据齐备。
不适用阶段记录 not_applicable_stage 与批准依据，不写 verified；不得冒充 required CI 的 skipped。
每条 AC 至少一个阶段；waiver 绑定 revision/criterion/阶段/目标 SHA 与人工授权，不自动跨 SHA。

AC ID 生命周期规则：

- 创建 Contract 时按顺序生成 `AC-001`、`AC-002` …；
- 仅修改某条 AC 的描述、验证方式或优先级时，保留原 ID 并在新 Revision 中记录变更；
- 删除的 AC 不复用其 ID，历史 Revision 中保持可追溯；后续新增 AC 使用更大的序号；
- 验收证据始终绑定 `(requirement_revision_id, criterion_id)`，不能跨 Revision 偷换。

`RequirementContract.dependencies` 在创建 Revision 的同一事务中规范化写入
`requirement_dependencies`，记录所属 revision；调度器只读取当前 revision 的规范化关系。
旧 Revision 的依赖记录不更新，不能先修改活动依赖再补写 Contract。

依赖分期：

- Phase 0 仅接受 `related_to`，用于展示和追溯；`blocks` / `depends_on` 在创建和 Ready 校验时
  明确拒绝，不得静默忽略或以 Submitted 当作满足。
- Phase 1 启用同 Repository 的执行性依赖，统一规范化为“下游依赖上游”并检测环。
  领取下游前，上游必须 `Done`，且其已验证 `merged_commit_sha` 必须是下游候选
  `base_commit_sha` 的祖先；fetch 后仍不满足则不领取、不消耗 retry。
- Phase 0 / Phase 1 只使用一个 Repository；Phase 2 开放多仓后，跨 Repository 仍只支持
  `related_to`。跨仓交付物、部署版本等执行性依赖另行定义，
  不以另一个仓库的 Submitted/Done 推断代码或服务已可用。

每次 Contract 修改都生成新的 `requirement_revision`，不能原地覆盖。AgentRun 绑定创建时的
revision 和完整 Agent Context canonical digest：

```text
Requirement
    → RequirementRevision
    → context_input_digest
    → AgentRun
    → Commit / PR / CI evidence
```

直接录入流程是：

```text
填写 Requirement Contract
    ↓
评审通过（POST /ready）
    ↓
冻结 revision 和 digest
    ↓
Agent 执行
```

ADR/OpenSpec 是否存在不影响执行路径；Requirement Contract 始终是执行和验收的唯一输入。

### 7.3 Discussion 与 Contract 草稿生成

降低评审成本的两个能力，按 Phase 2 → Phase 3 顺序启用：

**Phase 2：描述 → Contract 草稿 + 澄清 Agent。** 用户只写自然语言描述，平台调用一个只读
Agent 生成 problem / goal / in_scope / out_of_scope / AC 草稿，并为每条 AC 建议
`verification_method` 与 `verification_ref`（从仓库现有测试、CI Check 名称中选）。
澄清 Agent 随后对草稿挑刺：歧义、不可机器验证的 AC、与仓库现状冲突、遗漏的边界条件，
输出问题列表；用户回答后草稿更新。用户评审草稿并 `POST /ready`。澄清 Agent 不能自动 Ready，
也不能修改已 Ready 的 Revision。这是评审前的辅助，目标是让评审本身从"写"变成"改和确认"。

**Phase 3：Discussion。** 多轮对话式澄清，保存完整对话，从对话生成或完善 Contract：

```text
Discussion
├── conversation
└── optional requirement draft
```

Discussion 不是平台执行前提，也不是 Requirement 的状态来源；平台不要求把讨论转成 ADR 或 OpenSpec。

### 7.4 ADR / OpenSpec：仓库 Git 输入（可选）

ADR 和 OpenSpec 不属于平台一级实体，不进入平台自己的文档生命周期或状态机。

如果 Project/Repository policy 启用它们，约定直接放在代码仓库中：

```text
docs/adr/ADR-001-postgresql.md
openspec/authentication.md
```

Git 负责它们的版本和批准事实：

```text
每份 ADR 一个文件
新决策使用新编号
文件不原地改写
合并到默认分支才视为批准
```

Project/Repository 层的 `knowledge_policy` 只决定 Agent Context 是否读取这些仓库路径，
不解析语义、不维护专有文档版本状态：选中的文件按原文原样附加到 Agent Context，
由 Agent 自行理解。

```toml
[knowledge_policy]
enabled = true
max_files = 20
max_file_bytes = 100_000
max_total_bytes = 1_000_000

[[knowledge_policy.sources]]
type = "adr"
root = "docs/adr"
pattern = "*.md"
max_files = 10

[[knowledge_policy.sources]]
type = "openspec"
root = "openspec"
pattern = "*.md"
max_files = 10
```

限制（超限跳过并告警，不得静默截断文件内容）：

- 最多 20 个文件；
- 单文件最大 100KB；
- 总大小最大 1MB。

Repository 配置覆盖 Project 默认值。

Requirement 不需要保存 ADR/OpenSpec 外键。某次执行实际读取了哪些文件，由
`agent_context_snapshots.source_refs` 和 `context_input_digest` 记录。

纳入 digest 的文件统一按内容身份记录：

```text
已提交文件：
  path + blob_oid
  默认来源 = Repository canonical clone 的 canonical_base_ref

本项目本地开发的未提交文件例外：
  path + content_hash + source=local_uncommitted
```

`repository_commit_sha`、`workflow_hash` 等值是审计和输入来源元数据；digest 不把仓库
HEAD 当作所有文件的身份，也不使用 Agent commit、PR head 或 Recovery commit。

### 7.5 可追溯关系

必须保留：

```text
Requirement Source（可选）
    → Requirement Contract
    → Requirement Revision
    → Agent Context Snapshot
    → AgentRun
    → Commit / PR
```

这样可以回答：

- 这个 Requirement 来源于直接录入、Discussion、仓库文件还是外部事件？
- Agent 执行时冻结的是哪一个 Requirement Revision？
- Agent 执行时读取了哪些输入文件和 commit？
- 执行时 Agent 输入集合的 `context_input_digest` 是什么？
- 哪个 PR 完成了它？

### 7.6 输入漂移检测与重验证

这是防止“输入变了但 Requirement 仍显示完成”的强制流程。ADR、OpenSpec、普通文档、
Discussion 摘要和 Contract 在这里一律视为普通输入，不存在特殊事件路径。

#### 版本绑定

评审通过（Ready）时冻结输入；进入 Queued、开始 Run 或恢复交接时只验证已有冻结授权，
不能通过自动重算并覆盖冻结值来接受变化。快照保存：

```text
requirement_revision_id
context_input_digest
repository_commit_sha       # 审计元数据，不是文件身份
workflow_hash               # 审计元数据；文件身份仍是 blob_oid/content_hash
gate_policy_digest          # MVP 即单独强制校验；Phase 2 再纳入完整上下文摘要
source_refs                # 每个源的 repository/ref/commit/path/blob 身份
```

`context_input_digest` 定义为冻结时 Agent 输入集合的 canonical hash。计算规则固定为：

```text
1. 每个输入记录为 {kind, path/ref, identity}
2. 路径统一 Unix `/`，禁止 `..` 和绝对路径逃逸
3. 文件列表按 kind、path/ref 字典序排序
4. JSON 使用 UTF-8、无空格、固定键顺序序列化
5. 对 canonical JSON 计算 SHA-256
```

输入集合至少包含：

```text
Project/Repository policy
+ Requirement Contract
+ Acceptance Criteria
+ Constraints
+ Dependencies
+ 按 policy 选中的仓库文件内容
+ WORKFLOW.md 内容和 hash
+ Harness-Gate 配置和 policy digest
+ Agent 执行基线 ref（只用于确定读取版本和来源关系，不用 HEAD SHA 代替文件内容身份）
```

MVP 范围：

```text
context_input_digest(MVP)
= hash(Contract + AC + WORKFLOW.md + knowledge_policy 实际读取文件的
      path + blob_oid；本地未提交例外为 path + content_hash + source)
```

- MVP 计算 Contract + AC + WORKFLOW.md，以及 `knowledge_policy` 实际读取文件的
  `path + blob_oid`（本地未提交例外为 `path + content_hash + source`）；
- 已提交文件使用 `path + blob_oid`；MVP 默认从 canonical clone 的 `canonical_base_ref` 读取；
- 仅本项目本地开发允许使用 `path + content_hash`，并显式标记 `local_uncommitted`；
- Agent 产出的 `artifact_commit_sha`、PR head 和 Recovery commit 不属于输入 digest；
- Phase 2 再把完整 policy、Harness-Gate 配置和更丰富的上下文纳入 digest。

不进入 MVP 通用 digest 不等于不校验安全策略：受信 Gate policy revision、沙箱和工具授权
从 Phase 0 起单独绑定快照；策略撤销立即停止新的工具调用和交接，不能等 Phase 2 才生效。

#### 漂移触发

冻结之后，系统在重新调度、开始 AgentRun、提交产出和准备 `Done` 时重算当前输入 digest。
非本次产出的输入变化产生统一的 `InputSnapshotChanged` 事件；比较依据包括内容身份与
来源关系，而不是无条件将“当前默认分支内容”替换为该 Run 的原始输入：

```text
存在未经授权的外部输入变化
AND requirement.state NOT IN (Done, Cancelled)
    → InputSnapshotChanged
```

变化来源可以是：

- Requirement Contract 或 AC 修改；
- 关联依赖的完成条件修改；
- Project/Repository policy 修改；
- `WORKFLOW.md` 修改；
- Harness-Gate 配置或版本修改（Phase 2 完整 digest）；
- policy 选中的任意仓库文件修改；
- Agent 基线变化导致实际读取的文件内容变化；
- 外部来源被重新生成并实际写入 Contract 或选中的 Context。

Agent commit 和 PR head 是产出身份，本身不进入输入 digest。若本次 PR 修改 WORKFLOW 或
选中的知识文件，合并时必须执行来源分类：

1. 保存冻结源 commit/path/blob、封存 artifact 的对应 blob、实际集成基线和 merged commit。
2. 用 Git 变更来源核对冻结输入到集成基线间是否有外部变更；再核对最终文件变化是否确由本次
   已授权 artifact 引入。新增、删除、重命名文件同样处理，不能只比较 HEAD 或最终内容相等。
3. 可证明完全来自本次产出的变化记录为 `OutputIntegrated`，不使本次 Run 的输入失效；
   后续 Requirement 读取新的默认分支内容。
4. 外部变更、混合修改、来源无法证明或不在授权范围内的变化进入 `NeedsRevalidation`；
   不因属于同一 PR 就一律豁免。WORKFLOW/Gate 规则更新仍需独立的安全策略批准。

Run 的输入 digest 永不回写；用于最终资格判断的是冻结快照完整性、来源分类和显式重验证授权。

Phase 1 的完成资格按本次实际集成基线与 merged commit 判断；合并之后默认分支的新变化属于
后续需求，不使尚在等待最终验证的已合并产出反复失效。人工 Contract 修改和安全策略撤销例外，
仍立即撤销相应授权。

事件写入 inbox/outbox 后，由 Reconciler 处理，不需要识别变化来自 ADR、OpenSpec 还是普通文件。

Phase 0c 起不运行独立 `DigestReconciler`，由 Scheduler / Supervisor 在以下时机同步计算
（Phase 0a / 0b 冻结 Contract/策略身份，但不计算完整 context_input_digest；
通过 4.1 的受控输入替代处理修订，完整 digest 字段为 null 并记录 input_identity_version=0）：

```text
Ready 确认前
Agent start 前
提交产出 / 进入 Submitted 前
```

Phase 2 增加事件触发的 `DigestReconciler`，用于完整输入集合和跨 Requirement 影响分析。

#### 影响处理

漂移处理默认自动，只在自动路径失败时才需要人：

```text
检测到变化
    ↓
比较旧/新 digest
    ↓
计算受影响 Requirements
    ↓
Ready / Queued / Running / Submitted / WaitingCI / WaitingReview / Merging
    → NeedsRevalidation
    ↓
停止新的 Agent dispatch，终止旧执行组
    ↓
重新生成当前 Agent Context
    ↓
平台自动分类：
  ├── Contract / AC 变了（通过显示影响的 revise 命令提交，记录重新评审授权）
  │     → 自动生成新 Revision → Ready；已合并时自动创建 follow-up Draft 进待办箱
  ├── Contract / AC 未变，仅 WORKFLOW / 知识文件 / Policy 变化
  │     ├── 无有效产出 → 保全旧工作、生成新输入下的 RecoveryPlan 后自动回 Ready
  │     └── 有有效封存产出 → 在新输入下自动重跑验证
  │           ├── 通过 → 记录 auto_revalidated 授权，恢复原阶段
  │           └── 失败 → 待办箱：人选择"新执行"或"修改 Contract"
  └── 安全策略撤销（Gate policy / 沙箱 / 工具授权）
        → 立即停止，待办箱；不自动恢复
```

Repository Policy 开启 `require_manual_revalidation` 时（信任建立期），第二类也进待办箱由人确认
no-impact / affected；默认关闭。

MVP 状态机子集只涉及 `Ready` / `Running` / `Submitted`；`Queued` / `WaitingCI` /
`WaitingReview` 在 Phase 1 恢复完整状态机后适用。

no-impact/affected 的分类结果作为 `requirement_events` 持久化，记录操作者（平台或人）、理由、旧/新
snapshot、原 manifest、适用 revision 与授权 generation。Contract/AC 发生变化不能使用
no-impact 偷换 Revision；不变的 Contract 可以复用旧产出，但须重跑当前要求的验证。
已发出的旧交接调用先对账，新的交接使用新授权；不修改原 Run、声明或 manifest。

检测到漂移即撤销后续写入/交接授权，终止并确认旧执行组静止；旧 generation 的完成消息只能归档。
`NeedsRevalidation` 必须重新生成 Agent Context Snapshot，不能只改状态字符串。

已完成的 Requirement 不被静默改写；后续变化由用户创建 follow-up，或在 Phase 2 输入对账中
生成待确认的 follow-up Draft，不自动启动 Agent，不回写旧完成快照。

#### Phase 1 `Done` 前最终检查

平台只有在 Phase 1 及之后、以下条件全部满足时才允许 `Done`：

```text
PR 已合并
AND 最终验证批次指向精确的 merged_commit_sha
AND 该 SHA 的必要 CI 和受信 Gate 通过
AND 所有 post_merge 适用 AC 在该批次中 verified 或由允许的人工策略明确 waived
AND 所有仅 pre_merge 适用 AC 的原批次证据仍有效、来源关系已核查（不改标为 post_merge 执行）
AND Run 输入快照完整，来源变化已分类且必要的重验证授权有效
AND Policy 要求的 Review 证据齐备（默认 Policy 不要求人工 Review）
AND 没有未解决的 `NeedsRevalidation`
```

以上条件全部可由平台自动判定；默认配置下 `Done` 不需要任何人参与。

“必要 Gate”按 Repository Policy 判断：本项目强制 Harness-Gate；外部仓库可使用已批准的
custom/none 策略。none 不是伪造 Gate pass，必须显式记录未配置门禁；Contract 指定的
gate_check 或 ci_check 仍需各自证据，不能随 none 自动通过。

默认采用合并后验证，不把 PR 合并事件当作验证通过：

- 自动测试和 gate_check：平台建立 `post_merge` VerificationBatch，在隔离环境检出最终 SHA，
  用当前受信策略执行，不启动编码 Agent。
- ci_check：匹配 policy 中固定的 Check 名称/发布 App 与精确 SHA，等待默认分支 CI 结果。
  缺少该 SHA 的结果保持 pending；超时进入 Blocked。平台不默认申请 Actions 重跑写权限。
- 合并前的 PR head、预合并 SHA 和最终合并 SHA 分别存储；不同 SHA 的 CI 不自动继承。
- manual_review（仅 Policy 开启 `allow_manual_review` 时出现）：approval 必须来自允许 reviewer，
  未被撤销，明确关联 revision 和 AC。只有批准的源提交与最终提交的完整 Git tree 相同、
  且人工验收规则允许时，才可追加 `review_tree_equivalence` 证据；保留原 approval 的 SHA，
  不将其改标为最终 SHA。不满足条件时，对最终 SHA 生成待办箱项请求新的逐项人工验收。
- `waived` 必须有授权操作者、理由、时间和目标 SHA，且符合 Repository waiver policy；
  Agent 不能设置。普通 PR approval 不能无条件将全部 manual_review AC 标记通过。

每次验证追加一行证据：

```text
criterion_id
requirement_revision_id
verification_batch_id
agent_run_id                 # 合并后平台验证可为空
status = pending | verified | failed | waived
evidence_type
evidence_ref
subject_commit_sha           # 本条证据支持的目标产出
executed_commit_sha          # 实际运行测试的 SHA；人工证据可为空
source_commit_sha            # approval / 继承证据的原 SHA
source_evidence_id
verified_at
```

`ci_check` 在 Phase 0 保持 pending，Phase 1 起自动写入；只读 PR/CI 摘要不等于合格验收证据。
当前验收视图按
`revision + criterion + 目标 SHA + 所选验证批次` 派生，不能选取另一个 Run/SHA 的最近成功结果。

---

## 8. Agent Runtime

### 8.1 Runtime 抽象

Runtime 和 Model 必须分离。

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn start(&self, request: AgentRunRequest)
        -> Result<AgentHandle, RuntimeError>;

    async fn stop(&self, run_id: Uuid)
        -> Result<(), RuntimeError>;

    async fn status(&self, run_id: Uuid)
        -> Result<AgentStatus, RuntimeError>;

    async fn subscribe_events(&self, run_id: Uuid)
        -> Result<RuntimeEventStream, RuntimeError>;

    async fn send_approval(
        &self, run_id: Uuid, approval_id: Uuid, decision: ApprovalDecision
    ) -> Result<(), RuntimeError>;

    async fn send_user_input(
        &self, run_id: Uuid, input_request_id: Uuid, response: UserInputResponse
    )
        -> Result<(), RuntimeError>;

    async fn metrics(&self, run_id: Uuid)
        -> Result<RuntimeMetrics, RuntimeError>;

    async fn health_check(&self)
        -> Result<HealthStatus, RuntimeError>;
}

// Runtime trait 不绑定 Tokio mpsc；Supervisor 是每个 Run 的唯一 Runtime 事件消费者。
// RuntimeEventStream 可以由 Tokio、async-channel 或其他实现提供。
pub type RuntimeEventStream =
    std::pin::Pin<Box<dyn futures_core::Stream<Item = AgentEvent> + Send>>;
```

Supervisor 从该流读取一次后 fan-out 到三个下游：`agent_events` 持久化、Supervisor 状态机
以及 Phase 1 可选的 SSE broadcaster。其他组件不直接向 Runtime 重复订阅，避免单消费者
`mpsc::Receiver` 导致事件丢失或互相抢消费。

Phase 0 只实现：

```text
CodexRuntime
```

未来可以增加：

```text
ClaudeRuntime
OpenHandsRuntime
OtherRuntime
```

关键事件与数据类型：

```rust
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Started { run_id: Uuid, timestamp: DateTime<Utc> },
    TurnStarted { run_id: Uuid, turn_number: u32 },
    ToolCallRequested { run_id: Uuid, tool_name: String, args: serde_json::Value },
    ToolCallCompleted { run_id: Uuid, tool_name: String, result: ToolCallResult },
    ApprovalRequired { run_id: Uuid, approval_id: Uuid, action: String, reason: String },
    UserInputRequired { run_id: Uuid, input_request_id: Uuid, prompt: String },
    TokenUsageUpdated { run_id: Uuid, thread_id: String, usage: TokenUsage },
    TurnCompleted { run_id: Uuid, turn_number: u32 },
    CompletionDeclared { run_id: Uuid, declaration_id: Uuid },
    BlockerReported { run_id: Uuid, blocker_id: Uuid }, // 平台受控工具事件，不假定 Runtime 原生提供
    Completed { run_id: Uuid, outcome: AgentOutcome, total_tokens: TokenUsage, duration: Duration },
    Failed { run_id: Uuid, error: String, error_code: String },
}

pub struct RuntimeMetrics {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_microusd: i64,
    pub duration: Duration,
    pub turn_count: u32,
    pub tool_call_count: u32,
}

pub enum ApprovalDecision {
    Approve,
    Deny { reason: String },
}
```

MVP 审批策略：

```text
sandbox 允许范围内的命令执行和文件修改 → 自动放行，不产生请求
create_local_commit → 受控工具按 Run/路径/分支/HEAD 校验后执行
沙箱外请求（workspace 外路径、提权；网络另按 16.9）
    → 默认自动拒绝并写 agent_event；Agent 收到明确拒绝原因后自行调整
    → Repository Policy 可把指定类别（如已允许人工裁决的 workspace 外路径）改为 ask：暂停并进待办箱等单次批准
未知工具、访问平台凭证或其他 workspace → 拒绝，不提供绕过硬边界的批准
人工输入请求（elicitation / user input）→ 暂停并进待办箱；这是正常路径上唯一预期出现的人工点，
    Agent 只应在需求歧义或 AC 不可实现时使用
审批或人工输入超时 → Failed + manual_action_required = true
```

"默认拒绝而不是默认询问"是零介入的前提：一条需求跑到 PR 不应该因为 Agent 想 `curl` 一个未
白名单的地址就把人叫醒。S5 spike 用真实需求测出默认拒绝下的失败率，据此决定白名单初值。
Agent 提问（user input）不受此限制，因为它对应的是需求缺陷，属于 0.1 允许的介入类型。

MVP 默认 Runtime sandbox：

```text
thread_sandbox = workspace-write
network        = 域名白名单（见 16.9），由 平台全局 ∪ Repository 默认 ∪ 需求声明 生成
                 白名单外的域名由 Codex 本地代理直接拒绝，不产生审批请求、不打断人
```

审批策略必须绑定到 AgentRun 快照。普通配置更新仅影响新 Run；安全策略撤销会停止旧 Run，
不能以热更新方式偷偷放宽已有 session。

`workspace-write` 不等于 Git 元数据可写；默认 `.git` 及 worktree 的关联 Git 目录受保护。
S1 实测（23.S）表明这一保护只对 **app-server 进程 cwd** 顶层的 `.git` 生效，因此硬性要求：
每个 Run 单独 spawn 一个 app-server，进程 cwd 与 `thread/start.cwd` 都设为该 Run 的 worktree，
canonical clone 放在 worktree 之外。此时 worktree 的 `.git` 指针文件只读、真实 git dir 不可写，
`git add/commit` 在沙箱内直接失败。MVP 不通过关闭沙箱来解决本地提交，而是注册受控工具：

```text
create_local_commit(paths, message, expected_head)
```

LocalGitBroker 由平台维护，以当前 Run 的 generation/dispatch_epoch 校验授权，只接受当前
workspace 的明确相对路径；验证真实 worktree、目标 branch、expected_head 与 diff 范围。
使用固定 Git 二进制、参数数组和受信配置，禁用仓库自带 hooks、credential helpers 及未批准
filters；Gate hook 由独立隔离验证器执行。Broker 不持有 GitHub 凭证，不提供任意命令、rebase、
force push 或其他仓库写入接口。Agent 可请求提交，但不能直接修改共享 refs 或 canonical clone。
Phase 1 的确定性修复使用 Broker 的平台内部入口，按 DeterministicRepair ID、独立候选目录、
当前 Repository lease/epoch、RepairAttempt 和 Policy 白名单校验；不冒用已结束 Run 的租约。
此入口不注册为 Agent 动态工具，不接受任意命令；与 Agent/Handoff/Merge 写入者互斥。

审批/人工输入请求写入 `runtime_requests`，保存平台 UUID 与原始协议 request ID、Run/thread/turn、
generation、请求类型、不可变操作内容及摘要、过期时间、request_version 与状态；
决策记录 actor_id、client_kind（web/cli）、received_at 和幂等键，其中来源标签不是身份凭据。
回答先做 CAS 决策落库，再回复准确的原请求；
重复相同回答幂等，冲突回答、超时回答、取消后回答及旧 incarnation 的回答拒绝。
API 与 Web UI 从 Phase 0a（localhost）/ 0b（手机）起支持该闭环；轻量平台 CLI 是 0b P2 的
可选客户端。统一待办视图见 18.4，终端命令见 16.8。
Supervisor 是向 Runtime 回答该请求的唯一出口；所有客户端都不直连 app-server、
不写其 stdin、不修改 runtime_requests 表，也不自行执行获准的删除/网络等操作。

#### 人工等待规则（Phase 0a 起；0b 扩展手机，CLI 可选）

- UI 从持久化请求派生 `execution_phase=waiting_approval / waiting_input`，
  Requirement 仍为 Running、AgentRun 仍为 Running，不为此增加业务主状态。
- 只有持久化且当前仍 Pending 的运行时请求能够使普通 turn silence/stall 计时暂停；
  停止主动推进新 turn，未获批准的操作不得执行；已有并行调用仍受冻结策略和执行资源边界限制。
  通信丢失、进程退出、安全撤销和 lease 失效始终检测，不因“正在等人”而豁免。
- 人工等待单独使用 `human_wait_timeout`，首版默认 2 小时，按请求冻结 `expires_at`；
  active execution deadline 排除经验证的等待时长，但 Run 另有默认 8 小时绝对寿命上限。
  这些是平台默认值，不是对 Runtime 协议的假设，部署时须通过真实等待测试。
  等待策略绑定 Run 快照，不通过热更新延长已有请求；多请求并行等待时，各自独立过期，
  有效工作时长扣除等待区间的并集，不能重复扣除同一段时间。
- 等待时继续续租并保留 Agent 容量、Repository 写入权和 branch reservation；
  不为提高吞吐释放锁后继续使用原 session。首版队列明确显示“被人工等待占用”。
- 手机锁屏、断网、切后台、刷新、关闭页面、平台 CLI 退出或 Access 会话到期不等于拒绝/取消；
  登录并返回后从服务器重新拉取待办。客户端不离线排队批准，也不自动重放写请求。
- 审批单次生效，不扩展为全局允许；同一请求的第一次有效 CAS 决策胜出，其他设备显示已处理。
  接收决策与向 Runtime 回复分别记状态；回复结果不确定时按原 request ID 对账，
  不重发工具动作，也不把“决策已保存”假称为“执行已恢复”。
- 请求过期或 Run 绝对寿命耗尽后停止执行组、使所有未答请求失效并转人工，
  不自动批准、不自动新建 Run。重启不恢复旧 session：旧请求失效并提示人工重试，
  新 Run 必须产生新请求，历史批准不能复用。


#### 执行中阻塞声明（Phase 0a 起）

启动前预检不能覆盖运行中才发现的依赖/权限问题。平台注册受控 dynamicTool：

```text
report_blocker(reason_code, evidence_refs, required_conditions, progress_summary)
```

- `reason_code` 来自受限枚举；证据引用只能指向当前 Run 可访问的文件/工具结果，平台核对来源、
  大小和摘要。所需条件是声明性数据，不是 Agent 可要求宿主机执行的任意命令或授权。
- 工具携带 Run/generation/revision 及幂等请求身份；平台在同一事务保存 BlockerReport 和
  禁止续轮标记，再回复成功。与 report_completion 共享每 Run 的结束意图互斥守卫：先接受的
  意图生效，重复同一请求返回原记录，冲突/迟到请求拒绝。没有落库不能回复已受理。
- 受理后立即拒绝新写入工具和 turn/start，停止整个执行组并确认静止，然后按 3.7 保存工作快照。
  已有 shell 子进程不能靠拒绝工具继续运行。旧 Run 在确认退出后记录 Failed 与具体失败码；
  Requirement 在 Phase 0 进入 Failed + 原因，在 Phase 1 进入 Blocked；是否需要人工由下述
  recovery_mode 决定。不能把阻塞声明视为完成。
- 阻塞原因、未知分类与所需环境变化由平台映射处理，非瞬态且需人工处理时生成一次待办。
  网络范围外拒绝仍遵守 16.9：单次拒绝不产生待办；确实无法继续且需扩大范围时生成一次
  Contract 修订待办，不因调用本工具授予临时权限或自动扩大范围；
  不支持机器映射的原因保留原证据并按未知错误处理，不能猜测一个可自动重试的原因。
- 持久化 blocker 未解除时，Issue active、有新诊断文件、进程重启和重试余额都不能触发编码。
  自动解除或 Retry/Unblock 都必须按既有授权路径进入平台预检，追加满足解除条件的证据；条件未满足返回
  原 blocker，不调用模型。人工等待与安全撤销仍遵守原规则，环境修复不复用失效审批。
- 恢复时不复活旧 thread/Run；先生成 RecoveryPlan，根据 candidate/声明/manifest 分界恢复平台步骤或
  带 Recovery Context 的新 Run。Agent 必须用本工具报告无法继续的环境阻塞；自然语言
  “blocked”不能代替持久化协议。未按协议报告时保留两轮无进展/turn 上限的兜底，不承诺从
  任意自然语言自动识别全部阻塞。
- 工具回复丢失时以落库的结束意图为准，不续轮；若数据库/磁盘故障导致无法落库，则停止旧组、
  保留工作目录并关闭新调度，按存储故障恢复，不能退化为“没有声明，所以自动重新编码”。

**自动解除环境阻塞（Phase 0a 起）：** ProcessReconciler 消费持久化探测任务，由固定探针判断
依赖就绪、磁盘恢复、已批准服务恢复等条件。依赖预置/下载只执行 Policy 已批准的固定动作，
不扩大网络、文件权限或安装任意 Agent 提供的脚本。探测与恢复共用 blocker_id 的幂等记录，
默认最多 6 次探测、指数退避且最长等待 30 分钟；Policy 可冻结不同上限，重启不重置。
同一 blocker 只允许一个在途探测，达到上限后转一次人工待办，不无限轮询。

等待期间 Phase 0 使用 Failed + manual_action_required=false，Phase 1 使用 Blocked +
manual_action_required=false；UI 显示“等待环境自动恢复”，不进入人工待办箱。条件满足后，
平台追加解决证据并再次检查授权、待定人工请求、旧组静止、快照、输入资格和剩余预算，
通过后自动执行 RecoveryPlan；无需点击 Retry/Unblock。快照 pending 时先保存，不直接启动模型。
若已经因耗尽转人工，迟到探测结果仅归档；显式 Retry 可重新授权有限探测轮次，保留累计记录。
需要权限扩大、安全策略撤销、未知故障、人工问题未回答或旧审批失效时，recovery_mode=manual，
不能自动解除。权限原状由管理员恢复后，只有原先明确允许 auto_probe 的 blocker 才可自动继续。

Agent 运行完成必须有显式完成信号：

```text
report_completion(summary)
```

平台接受该工具时读取受控工作区 HEAD，并将声明与 Run/revision/digest/generation 原子持久化；
数据库提交成功后才回复工具成功。随后禁止新的写入工具调用，中断 session，确认整个执行组
退出，再确认 HEAD 未偏离声明、工作区无未提交改动、相对 change_base 有真实产出。
这些执行完整性条件满足后登记 candidate，再独立验证 Gate/AC。
对复用的中间 commit 必须校验来源并重新验证；不能仅凭历史 Gate 结果认定完成。
HEAD/工作树完整性确认并保全 candidate 后 AgentRun Succeeded；验证器在该不可变 SHA 的
独立检出上工作，Gate/AC 的结果写 VerificationBatch。只有通过才封存交接，不回写执行结果。
声明、产出清单和交接记录的绑定规则见 3.5，之后不再恢复这个 session 继续修改。
仅当没有持久化结束意图、待定人工请求或控制面存储故障时，才考虑下一 turn：
没有完成信号且工作区没有新变更，连续两次视为 `agent_completion_not_reported`；
有新变更则允许继续，直到 `max_turns`。已受理 blocker 优先于此判断，新增诊断文件也不续轮。

平台不会从自然语言结束语、进程退出码或现存 PR 推断完成。协议适配与验证见 8.3。

### 8.2 Codex Session 生命周期

```text
启动 codex app-server
    ↓
initialize → initialized
    ↓
thread/start（注册受控 dynamicTools）
    ↓
turn/start
    ↓
处理事件（notification / tool call / approval / token usage / turn completed）
    ├── report_blocker → 原子保存阻塞/禁止续轮 → 停止旧组 → 工作快照 → 等待解除条件
    ├── 人工请求待定 → 停止续轮，按独立等待规则处理
    ├── 无结束意图且存储健康/授权有效 → 按 8.1 判断是否同一 thread 继续 turn
    └── report_completion → 持久化声明 → 禁止新写入 → 停止整个执行组并确认退出
    ↓
平台确认声明 HEAD / 工作树完整性 → 保全 candidate → AgentRun = Succeeded / 释放执行租约
    ↓
独立 VerificationBatch：输入资格 / 受信 Gate / AC → 封存 manifest → HandoffOperation
    ↓
平台幂等 push / 查找或创建 PR → Submitted
```

### 8.3 协议边界

Codex app-server 的协议 schema 是协议唯一来源。实现不得仅凭本方案猜测字段。协议事实：

```text
- JSON-RPC 2.0 over stdio：每行一条 JSON（JSONL），线上省略 "jsonrpc":"2.0" 头
- 握手：initialize → initialized（每个连接一次）
- 原语：Thread / Turn / Item
- 流程：thread/start → turn/start → stdout 通知流（item/*、turn/completed）
- Token：thread/tokenUsage/updated，独立于 turn/completed；区分累计值和增量，防止重复累计
- 中断：turn/interrupt；turn 中断回执不替代执行组退出确认
```

实现规则：

- 参考 `symphony/elixir/lib/symphony_elixir/codex/app_server.ex` 的会话驱动结构；
  协议字段和认证隔离以锁定版本的 schema、官方文档与本方案安全边界为准。
- 协议类型用 `codex app-server generate-json-schema --out <dir>` 的产物 codegen 生成，
  禁止手写协议字段；`approval_policy` / `thread_sandbox` / `turn_sandbox_policy` 等值是
  pass-through 的 Codex 配置，以目标 app-server 版本的 schema 为准，不手维护枚举。
- 锁定 Codex 版本；升级流程 = 重新生成 schema + 重跑协议兼容测试。
- `create_local_commit` / `report_completion` / `report_blocker` / `read_observation` 使用 `dynamicTools`。按目标版本开启所需
  `initialize.capabilities.experimentalApi`，在 thread/start 注册 schema，并按协议处理
  `item/tool/call` 及原始 request ID 回包。codegen 必须包含所用实验类型；缺少能力时启动失败，
  不回退为解析聊天文字或授予通用 shell 权限。
- 支持 startup timeout / response timeout / turn silence timeout / stall timeout。
- 使用唯一请求 ID。
- 单独处理 stdout 协议流和 stderr 诊断流。
- Agent 子进程退出时返回明确错误分类。

CodexRuntime 验收标准：

```text
1. 能 spawn codex app-server（stdio），完成 initialize 握手
2. 能 thread/start + turn/start 驱动一条真实 prompt
3. 能消费消息流、turn/completed 和 thread/tokenUsage/updated，Token 去重累计正确
4. turn/interrupt 能中止进行中的 turn
5. 审批及 user input 能按 request ID 回复，覆盖并行请求、重复/超时/迟到回答
6. 子进程异常退出能分类为明确错误码（错误码表 agent_* 系列，见第 28 章）
7. 真实 worktree 中沙箱直接写 .git 被拒绝，create_local_commit 可完成受控提交（S1 已通过）
8. dynamicTools 注册/调用/回包有效；完成声明持久化后重启不会丢失或重复交接
9. report_blocker 落库后不续轮；与完成声明竞态、回复丢失、重启均只接受一个结束意图
10. read_observation 的 ID 授权、分页/摘要校验和不存在引用处理通过（8.5）
```

协议和安全参考（适配时以锁定版本复核）：

```text
https://developers.openai.com/codex/app-server/
https://learn.chatgpt.com/codex/agent-approvals-security
```

#### 兼容性核实记录

**2026-09-07（S1，真实运行）**：见 23.S 的 S1 结论。dynamicTools / `item/tool/call` /
`thread/tokenUsage/updated` / `turn/completed` 均在 0.153.4 上实测通过；沙箱可写根规则与
"每 Run 一个 app-server、cwd = worktree"的部署要求由此确定。

**2026-09-05（只读检查）**

本机 `codex-cli 0.153.4` 的只读检查结果：

- `codex --help` 和 `codex resume --help` 均列出 `--remote`，用于将 TUI 连接到远端 app-server；
  `codex app-server --help` 列出 stdio、Unix socket 和 WebSocket 传输选项。
- `codex app-server generate-json-schema --experimental` 生成的 ServerRequest 包含
  `item/commandExecution/requestApproval`、`item/fileChange/requestApproval`、
  `item/permissions/requestApproval` 和 `item/tool/requestUserInput`。
- 这些本机证据确认参数和请求类型存在，不证明原生 TUI 与平台可安全共同答复同一个请求，
  也不证明能直接接管任意已有 CLI 会话。本次未运行真实多客户端审批测试。
- 本次官方文档页面读取未成功，以上不声称为最新官方在线规格或所有版本的保证；
  部署仍须锁定并复核实际版本，不能只凭参数名就宣布兼容。

首版沿用平台独占的 app-server 控制连接；不把用户终端连接到该 Run 的原始控制 socket。
原生 TUI 直连若能绕过平台 CAS 落库、撤销检查或审计，即使能显示/处理审批也不符合本方案。
未来接入需验证同一 thread/turn 的请求归属、重复回答、断线/取消、权限与工具路由、
以及平台先落库再回复的顺序；如需实现协议代理，另行评估，不作为首版前提。

### 8.4 Agent Context

Agent Context 按层组装：

```text
Platform Context
    ├── Project policy
    └── Repository policy

Optional Repository Inputs
    ├── files selected by knowledge_policy
    └── source paths + blob_oid/content_hash identity

Execution Context
    ├── Requirement
    ├── dependencies
    ├── current branch
    ├── recent commits
    └── current PR

Failure Context
    ├── CI summary
    └── review comments

Recovery Context（每个恢复 Run 必需）
    ├── recovery_plan_id / resumed_from_run_id / work_snapshot_id
    ├── 当前输入身份与已恢复的文件/暂存状态清单
    ├── 已完成步骤及证据身份 / 尚未完成或 uncertain 的步骤
    ├── 阻塞与解除证据 / 已尝试动作 / 不应重复的已解决探针
    └── 下一动作、恢复阶段、预算余额与累计消耗
```

Context 必须有：

```text
context_version
source_refs
generated_at
redaction_status
token_estimate
```

**降级必须是可用的，而不是缺失的。** 上下文组装中任何一步失败（收据验证失败、文件超限、
模型摘要服务不可用）都不能让 Agent 拿到"半个上下文"或空章节：

```text
摘要/收据失败     → 回退为脱敏视图片段 + summary_unavailable 标记 + 授权归档引用
文件超限          → 跳过该文件并在 source_refs 标记 skipped，不静默截断内容（同 7.4）
policy 读取失败   → 拒绝启动 Run（这是安全边界，不是可降级项）
token 预算不足    → 按优先级裁剪 Optional Repository Inputs，保住 Platform/Execution/Failure/Recovery
                    必需层；裁剪结果记入 source_refs，可在 UI 看到"哪些输入没进去"
```

原则：**降级只降低压缩率，不降低证据可得性**。Agent 能通过 `source_refs` 里的引用
取回授权范围内的原始文件或日志脱敏视图；未脱敏敏感归档不向 Agent 开放；不允许出现"平台说有失败摘要但 Agent 看不到内容"的状态。
任何降级都写 `agent_events`，并计入 19.3 的指标（便于发现系统性失败）。

不要把全部 CI 日志、全部历史事件和全部 Repository 内容直接塞给 Agent。

恢复 Run 不能使用普通首次任务 Context 默默启动。平台从 3.7 的持久化记录机械组装 Recovery
Context，并在首次 turn 前保存 context snapshot；12.7 的 RepairContext 是其中的失败详情，
不能替代中间工作和恢复计划。Agent 自报“已完成”与平台验证通过分开标记，未核实的步骤为
uncertain；已验证步骤绑定输入和有效期，复用仍按 12.7.3，0a 最终验证不因此省略。
平台模板要求先核对恢复清单，再执行下一未完成动作；不要求重读整份规格或重做已经解决的
依赖探针，原始文档按引用读取。环境新鲜度检查由平台完成，不交给模型反复排查。

摘要不可用时降级为有上限的结构化清单、关键原始记录和 source_refs，标记 recovery_context_degraded，
保留阻塞/解除条件、快照身份、剩余工作和下一动作，不能降级成首次任务提示。关键恢复记录
缺失、快照不完整/损坏或身份不匹配时禁止启动恢复 Run，记录 recovery_state_invalid 并转人工；
不得自动丢弃既有工作。每个新 Run 的创建事件记录恢复来源或明确的首次执行/人工重新实施原因。

### 8.5 工具输出归档与分页读取（借鉴 ObservationPack）

Phase 0a 先覆盖平台可控制的验证输出、动态工具返回值及恢复上下文。长输出由平台保全，
返回有限大小的摘要/首尾片段、失败状态和稳定 observation_id，需要时通过受控工具读取：

```text
read_observation(observation_id, offset, max_bytes)
→ text, next_offset, eof, content_digest, original_bytes
```

这是平台新增的动态工具，不假定 Runtime 原生提供。观察记录包含 Run/验证 invocation、
工具调用身份、来源/内容摘要、创建时间、总字节/行数、脱敏视图及受控归档引用；offset 定义为
所返回视图的 UTF-8 字节偏移，平台限制单页大小并保持字符边界，提供 next_offset 避免跳读。
observation_id 只寻址已登记归档，不接受任意路径；每次读取检查当前调用者及恢复 Run 的来源授权。

- 原始执行记录和归档不可变；精简的是送入 Agent 的结果视图，不回写历史会话。首版用规则
  提取，不额外调用摘要模型；体积阈值、片段大小和分页预算由 Policy 配置并记录。
- 先成功归档并确认可读，再返回引用；归档失败则返回有上限的安全脱敏内容和 archive_unavailable/
  truncated 标记，不伪造可读取引用。普通压缩失败不使任务失败；必要验证/恢复证据未保全时，
  仍按 12.6/3.7 阻止封存或恢复，不能用 fail-open 放宽证据要求。
- 错误码、退出状态、未解决失败/uncertain、来源身份不可隐藏；12.3 的已验证失败收据、
  blocker 和 Recovery Context 的关键清单不二次压缩成普通首尾片段。收据每条引文仍可核对对应不可变脱敏视图，原始归档身份独立保留。
- 同一执行结果的重复投递按工具请求/结果身份幂等归档；不同执行即使字节相同也保留各自
  来源、时间和退出状态，不能因内容去重把旧观察当新证据。文件变化产生新观察，旧 ID 仍指旧版本。
- 归档登记为 10.6 的 ManagedResource；活动 Run、DiagnosticAttempt/待处理诊断卡、恢复计划和必需验收引用均持有资源引用。
  清理临时输出前须有可读归档，归档本身遵守引用与证据保留策略，不能在原 Run 结束时自动删除。
- Runtime 内置工具已有历史输出能否在后续请求中替换，须独立验证其公开上下文投影接口；
  0a 不修改 app-server 内部历史或用自建协议代理达成此功能。缺少接口时，只对受控工具的新结果
  和新 Run 的恢复上下文应用此机制，不宣称已获得 SoL-Pi 在 Pi 上的全部历史重放收益。

0a 验收：分页无遗漏/越权，原文摘要一致，摘要/归档失败显式降级，失败收据不二次裁剪，
新旧源码观察不混淆；原工作目录清理后，仍被恢复计划引用的观察可读取。比较实际输入量与
额外 recall 次数，不能只统计返回体变小就宣称净节约。

### 8.6 主动上下文压缩（Phase 2 可选实验）

借鉴 Online Context Compact 的经济性判断，不默认启用新的压缩控制器：预计剩余请求所节省
的输入成本，应足以覆盖摘要调用和缓存重建成本；接近窗口上限的容量保护与经济性优化分开记录。
缓存口径/剩余请求估计不足时不推测收益，继续使用 Runtime 自带机制。阶段边界可来自已完成
验证/实现步骤，不以存在 Phase 3 Planner 为前提；边界仅是候选触发点，不表示必然应压缩。

仅在真实长 Run 数据表明有收益空间、公开接口可用并通过 12.7.6 对照评测后启用。保留
Contract/授权身份、blocker、未完成动作和有效证据引用；压缩失败不能清空上下文。压缩完成
后的继续执行必须经过 Supervisor 的原有结束意图/授权/人工等待检查，不能绕过取消、阻塞、
完成声明或旧执行组静止要求，也不能通过压缩重置预算。该机制不替代 3.7/8.4 的故障恢复。



---

### 8.7 阻塞诊断与人工处理闭环

最小固定诊断、可操作待办和诊断包从 Phase 0a 必需；Phase 0b 增加手机通知/处理。
平台内诊断 Agent 从 Phase 0c 的只读隔离验收通过后启用，不能等到 Phase 4 多模型体系；
0a 若提前通过同等隔离与预算测试可提前启用，否则显示“固定诊断已完成，自动分析不可用”及
具体原因和下一步，不冒充已有模型诊断。以下能力均为设计要求，不代表当前平台已实现。

**流程：** 阻塞事实落库 → 停止冲突写入并保全工作 → 组装 DiagnosticBundle → 固定诊断 →
已授权恢复或有限只读诊断 → 更新同一个阻塞详情 → 必要时一次人工决策 → 平台验证解除条件 →
按 RecoveryPlan 恢复。页面先显示已知事实与诊断进度，不等诊断全部结束才显示阻塞。
没有 Agent 报告时，Supervisor/验证器/交接组件也必须从真实退出状态和阶段事件生成诊断包。
未知故障仍保持第 28 章 manual blocker 和停止执行的语义；只读诊断不等于自动解除。
页面立即可见，自动诊断窗口内可延迟需人工通知（最多首次固定诊断 60 秒加诊断窗口 3 分钟），
结束或不可用后按同一 blocker 发一次可操作提醒；不能因排队或诊断不可用无限静默。
已知的新授权/人工问题及紧急存储故障不等待模型分析才提示。

固定诊断只使用平台白名单探针，覆盖磁盘/可写目录、工具与已声明依赖、沙箱拒绝、
GitHub 认证/授权状态、远端 head/交接结果和证据/快照完整性。探针不读取或展示凭证值，
不执行仓库 hooks 或 Agent 建议的宿主机命令；每个探针有超时，整个首次诊断默认不超过
60 秒，与 8.1 的持续环境探测共用已有结果和身份，避免重复探测。不完整时也必须输出
已知事实、失败探针、缺失信息和下一项具体诊断动作，不能只有“unknown，请看日志”。
明确的人工问题、权限扩大、安全撤销直接给出已知待办，不为这些情况额外启动诊断模型。

**平台内有限诊断 Agent：** 仅当固定诊断不足、存在可用证据、旧写入组已静止、控制存储健康、
Policy 允许且预算可用时触发。默认 Policy diagnostic_mode=auto_when_unknown，能力未验证
不得开启；无需每次询问用户是否启动。沿用既有 Runtime/模型配置，输入只含 DiagnosticBundle
和必要脱敏分页引用，不重读整仓或历史会话。它不能修改代码、运行 shell/仓库脚本、任意联网、
访问其他 workspace/原始敏感日志、调用 Broker/完成声明/权限批准或自行解除 blocker。
这些限制须由实际工具和执行视图强制，不能只写在提示词里；模型认证隔离沿用 17.3。

默认每个 blocker 最多 1 次自动 DiagnosticAttempt、每个根需求最多 2 次；初始上下文默认最多
12,000 个估算 token，输出与分页另设可强制字节上限（输出 8 KiB、分页累计 64 KiB），最多 2 turn、
3 分钟（包含排队、启动和执行）。配置随尝试冻结；模型调用/token 另按实际可观察口径计量，
不把 turn 当作一次请求。
诊断次数在 root 行锁下先预留再排队；模型容量在实际启动前另于共享调度锁内领取，
两种预留分开记录。超时/崩溃/取消均保留已预留次数，不自动重新调用。
更换 bundle、重复消息、重启、Retry/Unblock 都不能重置同一 blocker 的自动额度。
诊断不扣 RepairAttempt，但有独立根预算，禁止借“诊断”进行编码或无限循环。

DiagnosticAttempt 的 Queued/Running/Completed/Failed/Cancelled 是执行记录状态，不使业务
进入 Ready 或 Running。诊断与编码共用全局模型并发额度（0/1 固定 1），由同一调度锁预留；
已停止但仍保留原目录的编码任务不会授权诊断写入。容量预留保存 holder/token/lease 与
执行组身份；租约到期必须确认诊断组静止才释放，重启不并行重开旧诊断。
排队只显示等待容量，不创建新业务需求；
只读诊断使用独立、无写入能力的证据视图，诊断结束释放模型额度，归档按 10.6 引用规则清理。
诊断队列不得无限抢占普通编码，取消/输入替代撤销未发诊断；在途诊断中断，迟到结果仅审计。

模型输出必须按 DiagnosisReport schema 校验，引用必须存在且属于当前 bundle/授权视图。
建议的动作只允许引用 action catalog；平台核查适用性后才显示按钮，模型结论不能将原因
升级为 confirmed，也不能改写 retry_class 或扩大授权。输出无效、模型不可用或预算耗尽时，
保留固定诊断结果与原始脱敏证据，标明诊断限制，不让“诊断失败”覆盖原 blocker 的原因。
对于安全类/权限类问题，即使诊断解释了原因，恢复仍走原有人工授权规则。

**仍需人工时：** 同一 blocker 更新一张待办，用户不必新开会话或重新描述任务。原因未知时，
明确列出已排除项、尚缺的具体证据，并给出一个范围最小的下一步（例如核实指定安装权限、
在指定主机执行批准的只读检查，或导出已有脱敏诊断包供人工分析）；不得只给通用 Retry 或日志链接。
确实需要用户补充信息时，在 blocker/version 下保存结构化回答及 actor 审计，不把回答送入
已结束的 app-server session，也不自动获得新的模型诊断预算。诊断包可下载，不含凭证，
已包含任务目标、进度、关键证据与未解决问题；外部分享仍需用户明确发起。

用户决策先经原有权限、版本、幂等和范围校验，再执行对应领域命令；页面依次显示“决定已保存”
“正在验证条件”“恢复到某步骤”。点击“已处理”不等于条件满足；验证失败在原卡片说明还缺什么，
不重复建待办、不盲目启动编码。未知原因不自动解除，不能用诊断模型的建议代替机器验证。

---

## 9. Workflow 和配置

### 9.1 配置分层

```text
平台默认配置
    ↓
Project 配置
    ↓
Repository 配置
    ↓
Requirement 覆盖
    ↓
AgentRun 快照
```

### 9.2 Repository WORKFLOW.md

保留 Symphony 的 repo-owned `WORKFLOW.md` 设计，用于保存：

- Agent Prompt
- Workspace hooks
- 测试和验证约定
- 代码库专属规则
- Agent 工作契约

建议路径：

```text
<repository>/WORKFLOW.md
```

平台数据库只保存：

```text
workflow_path
workflow_version
workflow_hash
```

Tracker Token、GitHub App 私钥和模型 Secret 不允许写入 `WORKFLOW.md`。

WORKFLOW 中的 prompt 可以作为输入快照；其中 hooks 和验证入口是可执行策略，必须绑定已批准的
源 commit/hash。Agent 产出的 WORKFLOW 变更只作为待批准修改，不在当前 Run 中执行。
安全策略批准和撤销不等于普通 Prompt 热更新。

### 9.3 多 Repository 配置

每个 Repository 都有自己的 Workflow：

```text
Project A / backend  → backend/WORKFLOW.md
Project A / frontend → frontend/WORKFLOW.md
Project B / api      → api/WORKFLOW.md
```

不能使用一个全局 `WORKFLOW.md` 覆盖所有仓库的测试命令、目录约束和 Agent 规则。

### 9.4 动态配置

允许动态更新：

- 调度间隔
- 全局并发限制（Phase 0 / Phase 1 固定 1；Phase 2 起可配置）
- 项目并发限制（Phase 3）
- Repository 并发限制（Phase 3）
- retry 上限
- Prompt 版本
- hooks
- 模型策略

MVP 仅启用以下动态更新：

```text
retry 上限、Prompt 版本、hooks 和模型策略（仍受安全审批和运行快照约束）
```

Project / Repository 容量与公平调度在 Phase 3 启用；运行中的 AgentRun 继续使用其启动时
的配置快照。hooks/验证入口变更必须先批准新的受信 policy revision；普通配置覆盖不能放宽
平台硬边界，安全撤销按 7.6 立即使后续调用失效。

不强制热更新：

- 正在运行的 Agent Session
- 已创建的 Worktree
- 已启动的 HTTP listener

非法配置必须保留最后一份有效配置，并记录错误。

---

## 10. Workspace 和 Git Worktree

### 10.1 Workspace 路径

多项目场景下不能只按 Requirement identifier 建目录。

推荐：

```text
<workspace_root>/
├── <project_id>/
│   ├── <repository_id>/
│   │   ├── <requirement_id>/
│   │   └── <requirement_id>/
```

实际使用稳定 ID，显示名称只用于日志和 UI：

```text
/workspaces/project-uuid/repository-uuid/requirement-uuid
```

### 10.2 Worktree 生命周期

```text
创建 workspace
    ↓
验证 workspace root containment
    ↓
git fetch
    ↓
git worktree add
    ↓
运行 after_create
    ↓
运行 Agent
    ↓
停止执行组并确认退出；after_run 只在隔离环境运行，不能改变已声明的提交
    ↓
验证并封存，交接与诊断保留期结束后清理
```

成功执行后的 workspace 按 Repository Policy 的保留期异步清理；默认保留 72 小时便于排障，
完成时即可入队，not_before 由保留策略计算。一次性测试资源不必等待整个 worktree 到期，见 10.6。

终态资源处置默认如下：

| Requirement 状态 | Branch / PR | Workspace / Worktree |
|---|---|---|
| `Submitted` | 保留 branch 和 PR | 默认保留 72 小时 |
| `Failed` | branch / 已有 PR 保留，便于重跑和排障 | 默认保留 72 小时 |
| `Cancelled` | 已 push branch 默认保留；已有 PR 不自动关闭，除非用户明确要求 | 默认保留 72 小时 |
| `Done` | PR 保留 | 进入延迟清理队列 |
| `Blocked`（Phase 1+） | branch / PR 保留，等待人工处理 | 保留至人工解除或保留期到期 |

清理 worktree 必须同时满足：无 active 执行组/lease/恢复屏障，必要产出和诊断已独立归档，
没有验证或交接操作仍依赖该工作目录，且距最后使用超过 `workspace_retention_hours`（默认
72 小时）。PR 保留不等于工作目录永久保留；历史 workspace 行记录已清理状态。
未解决 blocker 的唯一工作副本、pending/partial 快照及尚未恢复的必要文件不得按 72 小时
自动删除。清理原目录前须验证完整 WorkSnapshot 可读取且可还原；快照保留至恢复完成或
人工明确放弃中间工作，不能与原目录同时过期。
待交接的 sealed_ref、manifest 和证据不按工作目录保留期删除。不得仅因进程退出就删除
已 push branch 或 PR。

### 10.3 分支命名

普通 Requirement：

```text
ai/req-<short-id>-<slug>
```

CI Recovery：

```text
复用原 PR branch
```

ReviewFix：

```text
复用原 PR branch
```

只有用户明确要求独立修复 PR 时，才创建新分支。

### 10.4 Git 不变量

必须保证：

1. Agent cwd 是当前 Run 的 workspace。
2. workspace 位于配置的 workspace root 下。
3. Agent 不能直接在 main/default branch 上修改。
4. 一个 PR branch 同时只能有一个写入型 AgentRun。
5. Git 命令优先使用参数数组，禁止把外部输入直接拼进 shell。
6. 删除 workspace 前再次校验 canonical path。
7. Worktree、branch、PR 的关联写入数据库。
8. Git 元数据、remote 配置和 sealed refs 由 LocalGitBroker/平台管理，不向 Agent 开放写权限。
9. Gate 与 push 使用 manifest 中的固定 SHA，不读取可被修改的 branch HEAD 作为替代。
10. push 非预期拒绝时先 fetch 对账；只允许 fast-forward，14.4 的 lease 仅作 CAS 守卫。
    Phase 1 的授权内集成修复可新建候选保留远端历史；需要 rebase 时显式开始新执行，
    旧 manifest 和 Gate 证据不得套用于 rebase 后的新 SHA。

### 10.5 Git 失败分类

```text
git_fetch_failed
git_worktree_create_failed
git_branch_conflict
git_dirty_workspace
git_push_rejected
git_rebase_conflict
git_auth_failed
```

不同错误不能全部进入无限 retry。MVP 中冲突、权限和认证错误统一进入：

```text
Requirement.state = Failed
failure_code = ...
manual_action_required = true
```

Phase 1 起将对应需要人工处理的错误映射为 Blocked。

### 10.6 异步资源回收（Phase 0a 起）

任务结束只等待执行组静止和必要证据保全，不等待磁盘删除、容器/测试实例移除或保留期到期。
平台通过持久化 CleanupTask 交给独立后台 ResourceCleaner 消费，不由 Agent 记住清理步骤，
不调用模型、不占 Agent slot，也不因清理失败重新编码。0a 在同一 Rust 服务内运行独立异步
worker 即可；不要求新进程或外部 MQ，未来可拆成独立进程。

#### 10.6.1 资源登记与去重

平台管理的 workspace、验证检出、临时目录、测试服务/容器/数据库及构建目录必须登记
`ManagedResource`：resource_id、kind、owner_run_id/verification_id、resource_generation、
规范路径或 provider ID、创建意图/实际状态、当前引用/租约、保留策略和最后使用时间。
具体资源类型只随相应 executor/测试设施接入，不要求 0a 提前部署容器或数据库测试平台。
项目验证入口通过受控 runner 登记资源；Gate 可报告资源引用，但平台核查所属范围后才采信。
Agent 或测试脚本提供的任意路径/清理 shell 不能成为宿主机删除授权。

先保存创建意图，再创建资源；资源名/标签含稳定 resource_id，创建响应丢失时按 ID 对账，
不能再次分配同一逻辑资源。测试资源默认按 Run/VerificationInvocation 隔离命名，重试只复用
已确认健康且语义允许复用的实例，否则登记替代资源并回收旧实例。共享缓存/镜像/固定输入
有独立身份与使用引用，不因一个 Issue 完成就整体删除。平台不能承诺回收未登记的任意宿主资源；
本项目测试入口必须把所创建资源归入已登记目录或使用登记接口，违规产物作为资源审计异常。

#### 10.6.2 完成事件 → 持久化任务 → 后台回收

- 验证 invocation 结束、Run 终止、交接成功、Requirement Done/取消，以及资源最后引用释放，
  均评估清理资格。不能只监听 Done：0a 的成功终点是 Submitted，失败/取消也会留下资源。
- 在记录资源释放/终态的同一数据库事务中 upsert CleanupTask 和清理 outbox；完成事件与清理
  意图不能一个成功、另一个丢失。清理 task 的唯一键为 `(resource_id, resource_generation, action)`。
  任务包含 `not_before`、状态、attempt、next_attempt_at、claim_token/lease、错误与回收结果。
- 单仓 0a worker 默认并发 1、有独立 IO/CPU 限额；通过租约领取任务，实际删除不持有数据库长事务。
  在途状态与结果持久化，worker 崩溃后租约到期重新对账；重复消息和“删除成功但回执丢失”按
  精确资源身份幂等处理。已不存在的资源可记成功，不能借名称相同删除后来重新创建的实例。
- 删除前重新核查 owner/generation、路径或 provider 身份、执行组已退出、引用/租约、保留期、
  工作快照和证据保全。资格与资源领取必须互斥：CAS 标记 retiring 后拒绝新引用，实际回收
  只操作该实例；后续任务使用新的 resource_id。不能“检查无引用”后允许新任务再次占用再删除。
  文件系统删除不跟随符号链接，使用唯一实例目录并保持 retiring 期间路径占用；同名实例身份
  无法确认时停止清理，不能只按路径字符串重试。
- 仍有引用或保留期未到进入 Deferred，记录阻止原因并等引用释放/到期，不消耗失败重试次数。
  临时错误默认最多自动重试 3 次并退避，不逐次通知。永久权限/身份异常或耗尽转 Failed；
  按 repository + provider + failure_code/根因聚合一条运维待办，列出所有受影响资源；
  不重启 Agent、不回写已完成需求为失败。人工修复后一次命令可重新授权该聚合项内明确列出的
  task 做有限重投，保留累计审计；补漏扫描
  不得重建 task 或清零 attempt 来绕过失败上限。
- 0a 每 5 分钟扫描已登记资源与终态/引用记录，补建遗漏 task、处理过期 claim 和未确认创建；
  队列事件是主路径，定期对账是补漏。只扫描受管资源，不做全机目录猜测或全局 prune。

#### 10.6.3 清理时机与保留边界

| 资源 | 允许异步清理的最早时机 |
|---|---|
| 测试临时实例/目录、一次性验证检出 | 执行进程停止、必要结果和诊断已归档、无活跃引用之后 |
| 可重建的 Run 私有构建输出 | 无验证/恢复/后继消费者引用且策略允许后；不能误删作为验收证据的二进制 |
| Workspace / worktree | 满足 10.2 的全部条件后，默认最后使用起 72 小时；可先入队再延迟执行 |
| 共享缓存/镜像/固定依赖 | 最后引用释放且专属保留策略允许；单任务结束不能触发全局清空 |
| 工具输出归档/观察记录 | 8.5 的活动 Run/诊断/恢复/验收引用释放且保留策略允许后；清理临时原目录不影响仍有效的读取引用 |
| 工作快照、sealed refs、manifest、权威证据 | 不随临时资源清理；分别遵守恢复和证据保留规则 |

旧执行组终止和端口/服务写入冲突消除是安全交接条件，仍由 Supervisor/受控 runner 同步确认，
不能把仍会写文件或占用共享端口的进程丢给异步队列后立刻启动冲突任务。其余清理不阻塞
下一条符合依赖与容量条件的需求；若真实磁盘额度不足则正常施加调度背压，不承诺永远不等待。

资源清理策略由 Repository Policy 预先授权；受管临时资源与到期 worktree 的删除不要求逐次
人工确认。人工清理入口仍可保留；远端 branch/PR、共享镜像仓库、宿主机全局资源不在此授权内。
Phase 2 扩展多仓容量协调、共享缓存淘汰和历史数据 GC，不能把最小异步队列/重试/补漏推迟到 Phase 2。

#### 10.6.4 验收

0a 注入完成事务后进程崩溃、重复消息、删除成功但回执丢失、权限拒绝、后继任务持有引用、
同路径资源重新创建等场景：task 可补建/重领、不会误删新实例、失败可见且不调用模型。
演示上一条 Issue 的清理被故意延迟时下一条仍能在独立资源上运行；恢复快照未完整保存时
不能删除唯一原目录。对已登记测试资源核对创建/存活/待清理/已回收清单，不能只看 task 成功数。

---

## 11. GitHub 多仓库集成

### 11.1 GitHub App

使用 GitHub App，而不是为每个 Repository 保存一个高权限 PAT。

Repository 记录：

```text
github_installation_id
owner
name
default_branch
```

GitHub App 权限按最小集合申请（S2 实测，见 23.S）：

```text
必须（Phase 0 / 1 全流程已在 public 仓库验证）：
contents: write          push、读 commits/:sha/status、rulesets
pull_requests: write     找/建/合并 PR、mergeable_state
metadata: read

private 仓库建议加（未验证是否必需）：
checks: read             check-runs / check-suites
actions: read            actions/runs?head_sha=

不申请：
administration           branch protection 改用 rulesets API 与 pulls/:n.mergeable_state
commit_statuses          contents 权限已可读 combined status；平台不写 status
issues / deployments     仅对应功能启用时
```

installation token 有效期 1 小时，可按 `repositories` 限定到单仓库、按 `permissions` 向下收窄；
申请未持有的权限返回 422。Handoff Worker 在每次操作前检查剩余有效期，< 5 分钟则重新换取，
不缓存跨 Run 的 token。`GET /rate_limit` 对 installation token 返回 404，限额只能从响应头读。

Phase 0 / Phase 1 的 GitHub App 只安装到首个指定的个人仓库，Phase 2 才扩展安装范围。
首版只做 push/PR 交接和已关联 PR 的只读同步；不请求合并绕过或 Actions 重跑能力。

Webhook 不是普通 repository permission，而是 GitHub App 的事件订阅配置。Phase 0 不接入；
Phase 1 可选开启且不能替代周期对账。启用时至少订阅：

```text
push
pull_request
pull_request_review
pull_request_review_comment
check_run
workflow_run
check_suite
```

组织仓库权限留到后续版本。

### 11.2 Webhook 路由

本节仅在显式开启 Webhook 时适用，首版不需要暴露该入口。Cloudflare 路由隔离见 21.4；
GitHub 回调不经过交互式 Access 登录，独立入口仅接受经过 HMAC 验证的 webhook。

所有 Repository 可以共用一个 webhook endpoint：

```text
Webhook
    → 验证签名
    → delivery_id 去重
    → installation_id + owner/repo 定位 Repository
    → 定位 Project
    → 解析 PR branch / Requirement
    → 产生 Domain Event
```

Webhook 事件必须支持：

```text
push
pull_request
pull_request_review
pull_request_review_comment
check_run
workflow_run
check_suite
```

### 11.3 Webhook 幂等

建立 `webhook_events`：

```text
id
provider
delivery_id
event_type
repository_id
payload_hash
received_at
processed_at
status
error
```

`provider + delivery_id` 必须唯一。

Webhook 处理必须允许：

- 重复到达
- 乱序到达
- 延迟到达
- 处理过程中服务重启

### 11.4 PR 关联

PR 关联 Requirement 的优先级：

1. 数据库中的 `target_pull_request_id`
2. 分支命名中的 Requirement ID
3. PR body 中的机器可读 metadata
4. commit message 中的 Requirement ID

建议 PR body 包含：

```text
<!-- ai-factory:requirement=uuid -->
<!-- ai-factory:project=uuid -->
<!-- ai-factory:repository=uuid -->
```

PR body 是不可信输入，只能作为第 3 优先级的关联线索；数据库关联和分支命名优先级更高，
解析 metadata 时必须校验 UUID 格式、Repository 匹配和签名上下文，不能仅凭 body 改写
Requirement 状态。

### 11.5 首版只读同步与 GitHub 交接

Phase 0 的 GitHubReconciler 只轮询数据库已关联的 PR，不扫描整个账户：

- 默认每 60 秒批次读取开放 PR 的状态、head/base SHA、Checks/Actions 摘要和基础 Review 摘要；
  UI 的“刷新”先读取本地缓存，显式远端刷新按仓库节流并合并请求。
- GitHub 请求由后端持有 installation token 执行；手机不直接持有 GitHub App 凭证。
- 支持条件请求、分页、指数退避和服务端限流时间；持续失败显示 `stale/unknown`、
  `last_synced_at`、`sync_error`，不能把“读不到”当作 CI 成功、无评审意见或 PR 已关闭。
- PR head 改变后旧 CI 结果保留历史，但不展示成新 head 的通过结果；缺失检查显示 unknown/pending，
  skipped/neutral 不能无条件视为满足必需检查。CI 判据用 check-runs（`name` + `app.slug` 匹配，
  按发布 App、workflow/job、事件类型/受信 Check selector 分组，每组取当前有效 attempt），**不用 combined status**：只有 Actions check-runs 的仓库，
  `commits/:sha/status` 恒为 `pending`、`total_count` 为 0（S2 补测实测）。
- PR URL 来自适配器校验过的 GitHub 仓库/PR 关联，不从不可信需求文本任意打开外部链接。
- PR 创建、需要人工审查或发现合并/关闭时更新外部事实；重复轮询不会重复生成待办或通知。
  合并/关闭确认归档后停止高频轮询；首版每日至少低频复查一次终结 PR，也允许人工刷新，
  以发现重新打开。重开不自动启动 Agent，后续授权仍按 Requirement 状态机处理。
- Phase 0（L1）用户在 GitHub 网页审查和合并，平台只提供安全跳转，不触发自动合并，
  也不绕过仓库规则。Phase 1 起按 17.5 自动合并，本节的只读轮询升级为业务对账。

用户合并后，Phase 0 保留 `Requirement=Submitted`，同时显示
`PR=MERGED + merged_commit_sha + merged_at` 和“已合并，平台最终验收尚未启用”。
关闭但未合并明确显示“PR 已关闭，未交付”，不伪造 Done；后续修改需显式重新授权，
已合并的 PR 只能关联 follow-up Requirement。Phase 1 升级后由完整验证流程决定是否 Done。

Phase 0 的 CI 失败/Review 修改意见不自动把 Submitted 改为 Failed，也不假装已有自动修复。
用户可在手机修改需求并重新确认：生成新 Revision，按 7.6 经 NeedsRevalidation 撤销旧授权、
对账交接并重新 Ready；PR 仍开放时按 3.4 复用原分支。普通 Retry 按失败阶段恢复，
不用于绕过 Submitted 的重新授权。关闭但未合并的 PR 不自动重开，首版可创建新的显式需求。

---

## 12. CI 闭环

### 12.1 分期 CI 目标

Phase 0 读取已关联 PR 的 CI 状态和 GitHub 链接，供手机查看，不把它自动转成 AC 证据，
不分析完整日志、不生成修复任务。Phase 1 再接入精确 SHA 的验证与有限修复，按需读取：

```text
job
step
conclusion
url
sha
annotations
```

不一开始实现“理解所有 CI 系统”的通用分析器。

### 12.2 CI 状态流

以下业务状态流从 Phase 1 启用；Phase 0 只显示外部 CI 状态。

```text
PR Open
    ↓
CI Queued
    ↓
CI Running
    ├── Passed → WaitingReview
    └── Failed → Failure Analysis
```

### 12.3 Failure Summary（收据式，而非自由文本摘要）

发送给 Agent 的不是完整原始日志，而是一份**可机械核对的收据**。这一节的设计借鉴
NVlabs/SoL-Pi 的 Evidence-Preserving Reducer（见 23.S 的 S6 记录）：摘要是一组
**必须逐字节出现在不可变脱敏视图中的引文**，由平台验证后才交给 Agent。默认先消费门禁结构化结果、
CI annotations 和固定规则提取错误片段，不为每次失败额外调用摘要模型。只有规则提取不足，
且 Policy 显式启用并给出独立调用/输出预算时，才用摘要模型选择引文；模型选择沿用当前
Runtime Policy，不提前引入 Phase 4 的多模型调度。引用真实只证明对应脱敏视图中存在，不证明根因判断正确。

```text
CiFailureReceipt
├── schema_version         固定字符串，用于拒绝格式漂移
├── raw_source_sha256      原始日志归档哈希，仅供受限归档追溯
├── source_sha256          不可变脱敏视图哈希；绑定 redaction_policy_version
├── source_bytes / source_lines  脱敏视图的字节/行数
├── observed_is_error      平台观测到的退出状态（不是模型自报）
├── status                 success | failure；必须与 observed_is_error 一致
├── uncertain              bool；日志无明确失败信号时为 true
├── evidence[]             每项 {kind, line, quote, quote_sha256}
│     kind ∈ fatal | failure | warning | target | summary
│     quote 必须逐字节出现在该脱敏视图中，且不超过 MAX_QUOTE_CHARS
├── annotations            来自 CI API，非模型生成
├── relevant_files         平台按失败文件映射，非模型生成
├── log_artifact_refs      原始日志归档位置（平台私有存储）
└── possible_causes        由平台按 failure_code → retry_class 映射；**不由模型生成**
```

验证规则（任一不满足即整份作废，不做部分采信）：

```text
1. 能解析为 JSON 且 schema_version 匹配
2. source_sha256 == 平台脱敏视图归档哈希，source_bytes/lines 一致；原始归档另验 raw_source_sha256
3. status == failure 当且仅当 observed_is_error
4. evidence 数量不超过上限；每条 kind 在允许集合内
5. 每条 quote 满足：长度 ≤ MAX_QUOTE_CHARS 且脱敏视图 body.includes(quote) 为真，行号一致
6. 去重后仍至少有一条 fatal 或 failure 类证据（否则视为 uncertain）
7. 提取前先生成脱敏视图；仍检测到疑似凭证时拒绝该视图，生成新版本后重新提取/校验，不能改写原收据
```

失败降级（fail-open，与原设计一致）：

```text
收据验证失败 → 不做摘要，回退为"脱敏视图片段 + summary_unavailable 标记 + 授权归档引用"
              → Agent 仍能拿到证据，只是没被压缩
绝不返回：半份收据、被修改过的引文、模型自由发挥的 possible_causes
绝不因为摘要失败而把 Run 判为失败或消耗修复预算
```

其他要求：

- 最大字节数、Secret 脱敏、日志来源、commit 对齐、失败发生时间、是否允许自动修复
- 原始日志归档保留完整，不在摘要成功后删除
- 收据绑定 raw/view 双摘要与脱敏策略；模型仅能分页读取脱敏视图，原始日志保留于平台私有存储。
  安全脱敏失败时返回结构化错误码和受限引用，不回退泄露原文；原始证据仍供授权诊断。
- 收据关联验证批次和修复 Run（`failure_receipt_id` / `failure_receipt_source_sha256`），可追溯
- 平台侧不采信模型对根因的判断：`possible_causes` 与 `retry_class` 一律由 `failure_code` 映射
- 修复上下文按 12.7.4 组装；Phase 0a 用于本地声明后验证失败，Phase 1 才用于 hosted CI 修复

### 12.4 Recovery Requirement

Phase 1 起，显式启用修复策略的 CI Failure 才生成 Recovery Requirement：

```text
CiRecovery Requirement
├── root_requirement_id
├── recovery_source_type / recovery_source_id
├── target_pull_request_id
├── target_branch
├── parent_id
├── repair_attempt_id          # 总额读取 root，不在子任务重复维护
└── failure_summary
```

Recovery 默认：

```text
不新建 PR
不新建无关 branch
复用原 PR branch
```

串行化：

```text
同一 PR branch 同时最多 1 个 active Recovery / ReviewFix 或未完成交接
前一个执行及交接必须完成/终止并对账，才能开始下一个
避免并发修改同一 branch
```

### 12.5 修复预算

Phase 0 不创建 CiRecovery / ReviewFix，声明后的代码失败仍只允许执行预算内一次修复 Run。
Phase 1 起引入统一 RepairAttempt：覆盖声明后代码修复、CI 修复、ReviewFix 和确定性修复。
预算限制修复尝试，不把一次尝试误称为一次底层模型请求；模型调用、token 与工具消耗分别计量。

- 创建根需求时冻结 Policy，默认最多 3 次，允许配置 0～10；升级、重启、输入修订及 Unblock
  都不清零累计次数。模式默认 auto；require_fix_confirmation 仅在显式开启时请求人工。
- RepairAttempt 保存 root、sequence、failure source/version（local_validation / ci_failure /
  review_batch / remote_integration）、目标 SHA/配置身份、Policy、
  状态、formatter/Agent 执行引用和累计消耗。状态为 Reserved / Running / Waiting / Succeeded /
  Failed / Cancelled。先在 root 行锁内去重并预留额度，再发出工具或模型动作。
- total_recovery_attempts = 已预留 RepairAttempt 数；预留后取消也保留计数，不能靠反复取消
  或重建释放预算。同一失败事件的重复投递关联原尝试，不再次扣费。
- 每次尝试至多一次白名单 formatter 修复，之后仍有代码问题时至多启动一次逻辑编码修复，
  二者共享同一尝试。其基础设施恢复 Run 另受第 28 章执行预算限制；新一轮验证确认仍有
  代码失败则结束当前尝试，下一轮必须预留新 RepairAttempt，不能在子任务内无限修复。
- 声明后无 PR 的修复可以直接在原 Requirement 下启动新 Run，绑定 repair_attempt_id；
  PR 上修复通过 CiRecovery / ReviewFix 子任务承载。子任务数量不再作为扣费依据。
- CI 等待、验证基础设施重试、交接重试、只读 blocker 探针不扣修复额度，但分别有阶段上限
  与 deadline；不能用反复创建 batch/operation/子任务重置同一恢复链的预算。
- ReviewFix 等 CI 通过再运行，按 13.2 聚合尚未处理的意见版本；已有处理记录不重复唤醒。

耗尽时 root 进入 Blocked + manual_action_required，生成一次带证据与剩余工作的待办。
Unblock 只恢复仍有资格的步骤；到达修复上限只能人工完成当前修复或创建新的显式需求，
不能通过修改 Contract、追加子任务或 Unblock 无限增加自动额度。

### 12.6 本项目 Harness-Gate 门禁集成

分期口径：Phase 0a 的门禁是 Repository Policy 声明的 `gate_provider = custom` 命令（例如
`cargo test` / `npm test`），由平台在声明后的固定 SHA 上运行；本项目自身仓库在 Phase 0c 切换到
`gate_provider = harness-gate` 并启用本节全部规则。S4 spike 的结果决定是否可以提前。

Personal AI Software Factory 自身仓库使用 Harness-Gate 作为确定性工程质量门禁。它不替代
Codex harness，也不替代 GitHub Actions；它负责在本项目的本地提交、Agent 自检、PR、CI
和发布流程中统一执行可复现的质量检查。

参考实现：

```text
https://github.com/musutrade/Harness-Gate
```

初始接入基线固定到经过验证的不可变版本 **`v0.3.7`**（S4 实测版本）。本项目不得使用 `latest`、
`main` 或可变下载 URL；升级 Harness-Gate 必须显式修改版本并重新跑完整门禁。
平台必须记录**实际执行二进制的摘要**，不能只记版本号——本机曾出现 PATH 上是 0.1.0、源码仓库是
0.3.7 的情况，仅凭版本字符串无法发现。

#### 12.6.1 本项目仓库配置

本项目仓库在根目录维护：

```text
.harness-gate/
├── flow.toml
├── audit.toml
├── secrets.toml
└── reports/              # 仅本地开发报告；平台权威证据存储在 workspace 外
```

配置职责：

- `flow.toml`：component、scope、profile、step、超时和服务依赖；
- `audit.toml`：项目架构、分层和结构规则；
- `secrets.toml`：高置信凭据扫描规则；
- `reports/`：本地开发工具的输出目录，不是平台认定通过的可信来源。

本项目配置和报告只属于本项目，不作为外部 Repository 的默认配置。平台必须保存执行快照：

```text
executor_version          # 工具自报版本（仅参考，必须同时记二进制摘要）
executor_binary_digest    # 实际执行二进制的 sha256（版本字符串可被 PATH 上的旧版本骗过）
configuration_digest      # 工具原生字段，形如 sha256:<hex>（S4 实测）
source_identity           # git-tree:<sha> 或 working-tree:<sha>（S4 实测）
input_mode                # all | staged（S4 实测）
profile
trusted_gate_policy_revision_id
protected_entrypoint_digests
evidence_store_ref
invocation_id             # 工具生成，形如 inv-<ts>-<rand>-<pid>-<n>
```

Repository 使用通用的 `gate_provider`、`gate_config`、`gate_recovery_policy` 和受信 policy 指针；
`harness_gate_*` 字段只作为 AgentRun / Gate invocation 的不可变审计快照，不与
Repository 配置重复双写。

仓库内的配置是策略源，不是“Agent 当前工作区有什么就执行什么”。**S4 实测：`hook` 读的是 Git 暂存区
（index）里的配置，会物化到临时目录再校验，因此 Agent 改工作区配置不影响 `hook` 看到的规则；
但配置未暂存时 `hook` 直接报 `E1000` 失败**，平台必须在每次调用前确保配置已暂存、并检查退出码与
`TEST_SUMMARY`，否则门禁会静默失效。管理员从固定源 commit 批准 policy revision，
固定配置、强制步骤集合、工具版本和受保护验证入口的摘要。
Agent 修改这些路径时，变更保持待批准；不能自动把工作树的新摘要当作受信摘要。
批准需要鉴权 API、操作者和理由，Agent 工具没有策略批准权限。

**S4 补测证明这不是可选的加固**：保留 step id、把步骤命令改成恒真即可让 `hook` 报 PASS
（`config check` 也通过）。因此平台必须：① 持久化本 Run 应使用的 `configuration_digest`
（来自独立批准的 policy revision）；② 与该 invocation 记录里的值比对，**不一致即拒绝采信该次验证**；
③ 只把工具自报的 `TEST_SUMMARY` 当作辅助信息，不作为推进 `Submitted` / `Done` 的依据。

GateRunner 在独立验证检出中物化受信配置，并只读挂载到仓库相对路径；强制验证入口同样固定。
被测代码仍来自待验证的 artifact SHA，平台分别记录“被测源身份”和“验证策略身份”。
无法安全使用受信策略验证该变更时进入人工处理，不用修改后的配置自证通过。调用显式传：

```text
harness-gate
  --project-root <verification-root>
  --config <verification-root>/.harness-gate/flow.toml
  <command>
```

不得通过共享进程环境中的 `PROJECT_ROOT`、`HARNESS_GATE_CONFIG` 或其他隐式变量选择配置。
本项目自己的每个 invocation 必须有独立的 project root、报告根和 configuration digest。
权威报告由平台收集器写入 workspace 外的私有 evidence store；Agent 和仓库测试进程不能
修改报告清单、最终判定或历史证据。验证步骤以无平台凭证的低权限身份运行，参见 17.2。

平台管理的外部 Repository 可以声明自己的门禁命令或 GateRunner，但必须在
`Repository Policy` 中显式配置；未配置时不继承本项目的 `.harness-gate/` 文件。

#### 12.6.2 Agent 执行时机与仓库分层

普通受管 Repository（其 `Repository Policy` 显式配置 `gate_provider = harness-gate`）：

```text
Workspace / Worktree 创建
    ↓
harness-gate config check --format json
    ↓
harness-gate doctor --strict --json
    ↓
Agent 编码
    ↓
create_local_commit（明确 paths / expected_head）
    ↓
harness-gate hook
    ↓
受信 hook 通过后，由 LocalGitBroker 本地 commit
    ↓
Agent 调用 report_completion
    ↓
持久化声明 → 停止执行组 → 平台验证固定 commit / 输入资格 / 受信 Gate → 封存
    ↓
GitHub Adapter outbox：push → 查找或创建 PR
    ↓
Requirement = Submitted
```

MVP 的普通受管 Repository 必须对声明的最终 SHA 验证 hook 所要求的检查，而不是接受 Agent
报告的历史 hook 成功；Phase 1+ 封存前追加 `harness-gate verify --profile ci --all`。

Personal AI Software Factory 自身仓库（dogfood）使用更严格的顺序：

```text
Workspace / Worktree 创建
    ↓
config check + doctor --strict
    ↓
Agent 编码 → create_local_commit → 受信 hook → Broker 本地 commit
    ↓
report_completion 持久化 → 停止并确认执行组退出 → 平台校验声明 / HEAD / 输入资格
    ↓
harness-gate verify --profile ci --all
    ↓
封存 manifest → HandoffOperation：push 固定 SHA → 查找或创建 PR
    ↓
Requirement = Submitted
```

Phase 0 的唯一实际运行 Repository 是否为本项目自身仓库，由部署配置决定；无论是否
dogfood，Agent 都只通过受控工具请求本地 commit，push 和 PR 始终由平台交接流程代办。

Agent 不直接执行 push、创建/更新 PR 或调用 GitHub API。GitHub Adapter 必须先按
`repository_id + branch` 查找已有 PR，再决定创建还是更新；重复 outbox 投递不得创建重复 PR。

`doctor` 缓存必须绑定 12.7.1 的工具/环境/策略身份及有效期，不能只按 Worker 名称和配置摘要
复用，也不能跳过该执行环境的首次验证。Agent 不能
自行修改门禁结果、删除报告、覆盖失败码或把 warning 改写为 pass。

#### 12.6.3 本项目 CI 和 Merge Gate

本项目 GitHub Repository 的 Actions 至少运行：

```bash
harness-gate config check --format json
harness-gate doctor --strict --json
harness-gate verify --profile ci --all
```

CI 使用与平台一致的已批准 policy revision 和固定 Harness-Gate 二进制摘要。执行策略从受信
源获取，不直接信任 PR 对 workflow、配置或强制验证入口的修改；相关路径变更须独立人工批准，
GitHub App 不加入 branch protection bypass 列表。报告作为 Actions artifact 保存，至少包含：

```text
invocation_id
project_root / repository
input_mode
source_identity
configuration_digest
trusted_policy_revision
tested_commit_sha
selected_components
step results
failure codes
artifact manifest
```

GitHub branch protection 必须要求一个稳定的门禁检查，例如：

```text
Harness-Gate / verify
```

没有通过本项目 Harness-Gate，平台不得把本项目自身的变更推进到 `WaitingReview`；
若未来启用自动 Merge，也不得进入 `Merging` 或执行自动 Merge。

#### 12.6.4 Fail-closed 规则

下列任一情况都必须判定为门禁失败：

- 配置缺失、解析失败或配置 digest 与 invocation 不一致；
- 配置/强制验证入口与已批准 policy 不一致，或 policy 已撤销；
- secret scan 或 architecture audit 失败；
- 依赖步骤失败、超时、取消或被跳过；
- machine result 缺失、报告路径越界或 evidence 不完整；
- 未知 failure code / retry class；
- 报告、日志或 artifact 被替换、符号链接逃逸或无法验证；
- Harness-Gate 二进制版本不符合本项目 policy；
- GateRunner 进程异常退出或结果无法解析。

失败结果写入 VerificationBatch / ValidationEvidence，关联 AgentRun 用于追溯，不改变其
执行状态；Supervisor 按本节策略记录编排决定。`gate_failure_code` 直接用工具原生的 `failure_code`（S4 实测如
`SECRET_SCAN_FAILURE`、`TEST_SUMMARY: FAIL`），不另立一套命名：

```text
gate_status = failed
gate_failure_code        # 工具原生 failure_code
gate_retry_class         # 平台按 28 章映射
gate_report_path         # invocations/<invocation_id>/test_result.json
gate_invocation_id
gate_configuration_digest
gate_source_identity
```

门禁失败不生成新的普通 Feature。Phase 0 不生成 `CiRecovery`；Phase 1 起按下述策略自动修复
fixable 类失败，本项目自身仓库与外部仓库同一规则，安全类失败永远转人工。

#### 门禁失败后的处理策略

默认 fail-closed 表示失败时禁止提交/封存/交接，不等于失败就要人来。
处理优先级是安全硬规则、执行阶段、Repository Policy，最后才是错误码默认重试：

```text
所有阶段：
  config / secret / architecture / evidence / policy 撤销等失败立即停止，
  Phase 0 → Failed + manual_action_required；Phase 1+ → Blocked；永不自动修复

完成声明前的受信 hook：
  test / lint / build 失败作为 create_local_commit 的失败结果反馈给当前 Agent；
  不提交，允许当前 Run 在 max_turns / wall-clock 上限内修代码后再次请求；
  不消耗修复预算，不能自行降级门禁

声明后的平台验证（含该阶段适用 AC 的 automated_test）：
  test / lint / build / AC 失败 → 在修复预算内创建新 Run，把失败摘要作为 Failure Context
  （8.4）交给 Agent 重新编码，复用原分支；不重开已结束的 session
  Phase 0：执行预算内自动重跑一次，仍失败 → Failed + manual_action_required
  Phase 1+：消耗 12.5 的修复预算，耗尽 → Blocked
  纯验证基础设施超时仅重试同一固定 SHA 的验证步骤，次数见第 28 章

Phase 1+ PR 上的 CI 失败：
  gate_recovery_policy 默认 {"mode":"auto","allowed_failure_codes":["test_failed","lint_failed","build_failed"]}
  fixable 类失败自动生成 CiRecovery，复用原 PR 分支，消耗修复预算
  config_invalid / secret_detected / architecture_violation 永远 blocked
```

阶段内 hook 反馈、声明后重跑与 PR 上的 CiRecovery 是三条不同路径，均不能覆盖失败事实。
本项目 dogfood 与外部仓库使用同一策略；修改受信 Gate / workflow / 验证入口的变更不在
自动修复范围内，仍按 12.6.1 走独立批准。

#### 12.6.5 与 Symphony 的职责边界

```text
WORKFLOW.md
    = Agent 工作提示、hooks、仓库专属行为约束

Harness-Gate
    = 确定性 secret/audit/test/build/报告门禁

Codex App Server
    = 模型、工具、上下文、审批和文件修改执行

GitHub Actions
    = 远程可信执行环境和 Branch Protection 检查

Platform
    = Requirement、调度、状态、重试、对账和证据归档
```

Harness-Gate 的 capability/adapter 校验不是操作系统级 sandbox。即使本项目门禁通过，也
不能宣称 Agent 已获得完整系统隔离；不可信外部 Repository 仍需独立用户、容器或后续 OS
sandbox。

### 12.7 确定性流程接管与模型调用节约

依据：2026-09-13 对本机 Symphony 开发 Harness-Gate 的日志、交接台账与恢复快照复盘，
详见 [`docs/symphony-harness-gate-retrospective-2026-09-13.md`](docs/symphony-harness-gate-retrospective-2026-09-13.md)。
历史包含人工恢复和计数重置，不以台账 done 数宣称零介入率，也不预设 token 节省比例。

目标是在 AC 和验证强度不变的前提下减少模型调用、重复上下文和重复执行。职责固定如下：

| 流程 | 责任方 | 启动/继续编码 Agent 的条件 |
|---|---|---|
| 环境准备、能力预检、资源检查 | CodexSymphony，消费门禁 doctor 等探针 | 实际执行环境满足前置条件 |
| 固定检查、格式修复、检查依赖与证据复用资格 | 门禁/项目验证工具 | 确认剩余问题需要修改代码 |
| 是否调度验证、等待 CI、错误分类与恢复预算 | CodexSymphony | 失败属于允许的代码修复类型且预算未耗尽 |
| 本地提交、封存、push、PR、merge、远端对账 | CodexSymphony | 仅按既有政策处理需改代码的问题；交接故障不重启编码 |
| 需求理解、设计取舍、代码与测试修改 | Agent | 接收明确任务或带证据的修复包 |

Harness-Gate Result 仍是 Validation Evidence，不直接改 Requirement/AgentRun 状态。
平台不重写质量指标、阈值或检查依赖语义。下述复用与自动格式修复是待接入能力要求，
不假定当前固定版本 Harness-Gate 已支持；能力不可用时运行完整声明检查。

#### 12.7.1 真实执行环境预检（Phase 0a）

- Repository Policy 声明任务所需工具版本/能力、固定依赖及摘要、构建/临时目录、网络范围和
  验收执行器。平台在启动模型前，以与 Agent 相同的执行身份、沙箱和有效环境执行固定探针；
  host 上通过不能替代沙箱内通过，准备过程也不得扩大原有授权范围。
- 探针至少检查可写构建/临时目录、LocalGitBroker 可用性、已声明依赖/二进制能力及磁盘额度。
  `.git` 对 Agent 只读是预期边界，应验证平台提交工具，而不是要求 Agent 复制元数据或改用 API 发布。
- 固定输入可由平台预置并验摘要；缺失依赖、工具能力不匹配、权限错误不调用模型反复诊断。
  临时克隆/下载故障只重试准备步骤；确定性缺失按第 28 章记录不可执行原因并进入对应异常处理。
  不消耗编码/代码修复预算，准备本身有独立尝试上限和 deadline，禁止无限轮询。
- 预检记录绑定工具二进制、依赖锁定信息、沙箱/环境配置和策略 revision，保存检查时间与有效期。
  绑定输入变化或有效期届满时重新检查；磁盘/网络等易变条件在调度或使用前重查。
- 资源不足时暂停领取；运行中检测到持久化失败必须阻止后续外部写入，恢复后先对账。
  最小磁盘保护与 10.6 的单仓异步资源清理从 0a 开始；完整备份/隔离仍属 0c，
  多仓资源协调和历史数据 GC 留到 Phase 2。

#### 12.7.2 平台验证与确定性修复

Phase 0a 闭环固定为：预检 → Agent 编码/受控提交 → 完成声明落库 → 停止执行组 →
平台验证固定 SHA → 通过则封存交接；只有允许的 test/lint/build/AC 代码失败才按既有预算
启动带修复包的新 Run。验证超时重试验证，push/PR 故障重试交接，均不重新编码。
Agent 在编写期间可按需做针对性自检；平台不要求它重复代跑声明后验证或等待 CI。

Phase 1 的 CI 等待、结果轮询、annotations/失败日志读取、失败分类及合并均由平台完成。
基础设施故障不创建 CiRecovery；只有证据支持且 Policy 允许的代码失败才唤醒修复 Agent。
同一 PR/head 的重复失败事件只关联已有修复任务；未知失败按第 28 章 fail-closed。

Phase 1 可启用 Policy 白名单中的确定性 formatter 修复：固定工具、参数、路径范围和次数
（每个输入默认最多一次）。门禁执行修复命令，平台负责写入隔离和产物身份；须确认无其他
写入者，在单独可写候选检出保存前后 diff，通过 Broker 形成新 commit/产出记录。不得修改
已封存 SHA、受信配置、验证入口或历史报告，也不能把一般 lint/build 失败都当成可自动修复。
候选 SHA 重新进入正常验证/封存流程，旧 SHA 的证据不能证明候选通过；无需人另行确认已由
Policy 授权的格式变更。工具不支持或一次修复后仍失败时，返回证据并按原修复政策处理。
平台记录候选的产出来源为确定性修复及其 Policy 授权，不伪造 Agent 的完成声明；新旧产物
关联和新验证批次必须持久化。formatter 是同一 RepairAttempt 内的先行步骤（CI 修复可由 CiRecovery 承载），
仍计入根修复预算，不能借工具修复绕过 12.5；格式修复模式关闭时走原有代码修复路径。

#### 12.7.3 验证去重与受信证据复用

Phase 0a 先实现同一 VerificationBatch/step 的请求幂等：重复工具请求、重启或消息投递
不得创建并行的相同验证；执行结果未知时先核查旧执行组和结果，不把“执行过”当作通过。
0a 不跨批次缓存通过结论，仍在声明后的固定 SHA 上执行完整的 Policy 检查。

Phase 1 才启用经能力验证的证据复用。门禁负责声明检查依赖、适用范围和可复用性；平台验证
身份、证据完整性及策略有效性后决定是否调度。至少绑定：

```text
source_identity / external_input_digests
config_identity / trusted_policy_revision / protected_entrypoint_digests
executor_binary_digest / dependency_lock_digest / execution_environment_digest
profile / step / scope / validation_stage
evidence_ref / evidence_digest / produced_at / expires_at
```

仅当 required 检查被完整覆盖、身份兼容、证据可读取验证且未过期/撤销时可复用，并新建
引用原 invocation 的审计记录，不伪造“本次执行”。初版只允许完整身份相等的复用；源码改变
后默认重跑。基于变更范围的增量选择，须有门禁提供且经过验收的依赖/影响分析，否则不启用。
Agent 无权自行省略 required 检查。时间、外部服务或未固定依赖相关检查默认不可缓存，
只有显式固定输入与新鲜度规则才可例外；不确定是否有效时重新执行。

本地自检不能代替 required hosted CI，PR 证据不能代替 post_merge 验证；skipped、N/A、
shadow 或 synthetic 结果不得升级成必需验收通过。N/A 必须由版本化适用性规则判定。
失败证据可以用于去重和诊断，不允许当作成功缓存。

#### 12.7.4 修复包、重复失败与恢复记录

默认以规则和门禁结构化结果组装 `RepairContext`，包含当前 AC、源码/策略身份、失败
check/step/code、相关位置与必要错误片段、已尝试修复、已解决环境问题、剩余 AC、预算
及原始证据引用。证据片段遵守 12.3 的校验、脱敏和大小上限；不默认注入整段日志或旧会话。
Agent 可经受控读取按需获取原始证据，裁剪不得丢弃未解决失败或把 uncertain 变成通过。

Phase 1 对平台可观察的重复执行计算失败指纹：源码/配置/环境身份 + 规范化命令/检查
身份 + failure_code + 稳定错误特征。默认同一指纹连续失败两次且没有相关输入变化后，
停止第三次相同的自动执行或模型恢复，转一次可处理异常；确定性缺失按 12.7.1 首次即处理。
消息重复投递不算新失败次数，瞬态故障按第 28 章阶段预算/退避，不能用此规则误杀正常等待。
持续输出 token 不等于进展；新代码、新环境或新证据允许重新评估，但不清零累计预算。
只对可确认相同的执行使用该规则，未观测到的 Agent 内部行为不能凭推测判为无进展。

Phase 0a 起持久化恢复事件，记录原阻塞、解决证据、适用输入、恢复阶段、剩余工作及授权
来源。Retry/Unblock 不覆盖历史 attempt/token/失败记录；显式新授权另记增量额度，并保留
原累计值，不能放宽 12.5 的根需求修复上限。恢复前重新核查条件，不能只因旧摘要称“已解决”
就跳过预检。恢复仍遵守 candidate/声明/manifest 分界；无有效 candidate 来源链及 manifest 的中间 commit 不自动交接。
运行中阻塞必须使用 8.1 的 report_blocker；未解除前禁止续轮/新 Run。无声明恢复依赖
3.7 的完整工作快照和 8.4 的必需 Recovery Context，不把“新线程”实现为“从头重做”。

#### 12.7.5 验收与节约口径

Phase 0a 必测：缺依赖/只读 target/工具能力不匹配时模型调用为零；Agent 的 `.git` 只读
不影响 Broker 提交；重复验证请求不重复执行；push 响应丢失后只对账交接、不新建 AgentRun；
注入 ENOSPC 后不继续外部写入；有声明/manifest 时恢复平台步骤，无声明时只允许从
已保全的工作快照启动恢复 Run；依赖补齐后按原阶段恢复且累计记录不重置。
追加运行中 report_blocker：即使 Issue active 或新增诊断文件，受理后新增 turn 数为零；
模拟回复丢失/重启仍不续轮。恢复包摘要失败使用结构化降级，快照缺失/损坏则模型调用为零。

Phase 1 必测：相同身份的可复用验证无需新执行；源码、配置、环境、有效期或策略资格变化
使相应证据失效；formatter 改动生成新 SHA 并重验证；确定性相同失败不产生第三次自动执行；
CI 等待/交接重试不调用模型，代码失败才生成预算内修复任务。检查 skipped/N/A 和本地证据
不能替代 required CI/post_merge。修复包须保留必要失败证据，原始日志仍可受控读取。

对同类、同验收强度的已验收任务比较第 19.3 节指标，同时报告失败、人工介入与未完成任务。
不以降低覆盖率、跳过检查、遗漏失败或把工作转给人工来宣称节约。

#### 12.7.6 受控动作合并与逐项对照评测

Phase 1 P1 评估 Action Fusion：把一次已授权编辑与其可预测的快速检查合并为一个受控工具
请求，省去中间仅用于选择检查的模型决策。若 Runtime 现有批量工具已经完成同一序列，先测
是否还有收益，不为工具数量或形式单独新增接口。

适用实现必须满足：检查引用来自平台配置的 check_id/固定参数，不接受模型指定宿主机任意
命令；编辑与检查均在当前执行授权和沙箱内。编辑失败不执行检查；检查失败保留修改，分别
返回两个步骤的结果。按请求幂等键识别重放，重复请求不再次修改文件；检查绑定实际测试的
工作树身份，存在并发写入、取消或无法确认原执行结果时不得盲目重放。验证产物登记到 10.6。

该结果只提供开发反馈；即使快速检查通过，仍必须经完成声明后的独立固定 SHA 验证和 required
CI。模型选择开发期快速检查不构成最终验收权，Action Fusion 本身也不必然与受信门禁冲突。
不把每个微小编辑都触发全量测试作为默认行为。首版不替换 Runtime 内建编辑工具，新增接口
只有在公开扩展能力和隔离/幂等性验证通过后才启用，否则沿用现有工具调用。

每项效率机制独立开关并记录到 Run Policy 快照；先单项对照，再评估组合。Phase 0a 对 8.5
归档结果视图做小规模基线对照，Phase 1 对动作合并等新增机制做相同要求：冻结任务起始 SHA、
Contract、模型/配置、环境与验收器，保留失败和未完成任务；开发调参任务与最终验收任务分开。
最终验收结果不反复反馈调参后仍宣称为独立验证。

对照至少记录模型请求/turn 可观察性、输入/缓存/输出量、recall 和摘要额外调用、验证覆盖、
成功/失败/介入、实际成本口径和端到端耗时；在基线噪声较大时重复配对运行并报告样本数与波动。
必须证明机制实际触发；工具/UI 声称“省一次调用”或静态估算不等于实际请求减少。原有必需
AC 和安全回归逐项通过，任务能力无已确认退化，至少一项实际效率指标改善，才可默认开启；
样本不足则保持实验开关，不宣称稳定收益。组合也要独立回归，不能把单项收益直接相加。
SoL-Pi 的公开百分比不作为本项目验收目标；其组合平均分约保留 Pi 的 94%，不能作为严格
零质量损失的证明（来源见 23.S 的 S6）。


---

## 13. PR Review 闭环

### 13.1 Review 事件

Phase 0 只展示基础 Review 摘要和 GitHub 链接；以下逐条意见、修复触发和线程状态处理
从 Phase 1 启用。完整 diff 审查仍可交给 GitHub，不要求首版自建审查器。

必须区分：

```text
Review summary
Inline review comment
Review thread resolved state
Bot comment
Human comment
```

不是每条评论都应自动生成 ReviewFix。

### 13.2 ReviewFix 生成规则

满足以下条件的 review thread 自动进入 ReviewFix，不需要人选定：

- thread 来自 Policy 允许的 reviewer（人，或已配置的自动 reviewer bot）
- thread 未 resolved，且不是纯通知 / 已标记 ignored；该意见版本尚未被处理，也未被在途尝试领取
- PR 仍然开放，且 CI 已通过（先修 CI 再修 Review）
- 没有其他 Recovery / ReviewFix 正在修改同一 branch
- 修复预算（12.5）尚未耗尽
- 生成时核对意见来源 SHA 与当前 head；落在已被后续提交改掉的行上的意见标记 `stale` 并跳过，
  不静默套用到新版本；stale 意见在 ReviewFix 摘要中列出供 reviewer 知情

同一 PR 上同时存在的多条合格 thread 聚合为一个 ReviewFix，Agent 一次处理；
ReviewFix 完成后由平台在对应 thread 下回复"已在 <sha> 处理"，不自动 resolve 人写的 thread
（resolve 权留给 reviewer；自动 reviewer 的 thread 仅在其复审确认通过、且 Policy 授权时自动 resolve）。
平台保存 review_fix_items：repository/thread、意见事件版本（新评论 ID 或编辑内容摘要）、
source_sha、RepairAttempt、处理 SHA、证据、回复 outbox 与 waiting_review/resolved 等状态。
去重键为 thread + 意见版本；head 改变、线程尚未 resolved 或回复响应丢失均不能制造新意见版本。
已处理版本只等 reviewer 确认，不能再次调用 Agent。只有新的可执行意见或 reviewer 明确重审
指出问题仍存在时才申请新尝试；仍受根预算约束。新 head 本身不证明旧问题已解决。
RepairAttempt 在修复产物交接和针对该 SHA 的适用验证完成后结束；Review 的人工 resolve 等待
单独记录，不把它继续算作执行中修复。子 Requirement 的最终业务 Done 可随根验收后判定，
17.5 的“无进行中修复”检查实际 RepairAttempt/执行/交接，不等待子 Requirement 先 Done，
避免“等合并才能完成修复、等修复完成才能合并”的循环。
Policy 要求 resolve 而 reviewer 长期未处理时，默认 24 小时后聚合一次提醒；等待不消耗 Agent
容量。ignored/stale 意见保留审计，不自动 resolve 人类线程，不假称业务验收通过。

Policy 开启 `require_fix_confirmation` 时，ReviewFix 创建前进待办箱由人确认；默认关闭。

ReviewFix 记录：

```text
source_review_thread_ids[]
target_pull_request_id
target_branch
root_requirement_id
parent_id
review_items[] { thread_id, review_text, file_path, line, source_sha, stale }
```

### 13.3 ReviewFix 执行

```text
PR Review Threads（未 resolved）
    → 聚合 / stale 过滤
    → ReviewFix Requirement
    → 原 PR branch lease
    → Agent 修改
    → Commit / Push
    → CI
    → 平台在 thread 下回复处理位置 → 等待 resolve / 自动 reviewer 复审
```

ReviewFix、CiRecovery 与声明后修复共享 root 级 RepairAttempt 计数（默认合计 3，
Repository Policy 可配置；见 12.5），超限后 root Requirement 进入 `Blocked` 并生成一次待办；
Unblock 不增加修复额度。

---

## 14. Persistence 设计

### 14.1 PostgreSQL

从 Phase 0 起使用 PostgreSQL 保存状态、审批请求和恢复事实，不为首版另建临时存储实现。

Redis 和独立 MQ 不作为 Phase 0 / Phase 1 必需依赖：

- PostgreSQL 负责状态、锁、租约、事件历史和 retry。
- SSE 可以由应用进程通过数据库事件和 Pub/Sub 推送。
- 只有多 Worker、多机器或高事件吞吐时，再引入 Redis Streams/NATS。

### 14.2 核心表

```text
projects
repositories
repository_workflows

requirements
requirement_revisions
requirement_dependencies
requirement_acceptance_checks
verification_batches
requirement_events

discussions
discussion_messages

agent_runs
agent_events
agent_context_snapshots
observations                 # 8.5：来源、内容摘要、脱敏视图和受控归档引用
completion_declarations
artifact_candidates
deterministic_repairs
repair_attempts
repair_attempt_sources
review_fix_items
blocker_reports
diagnostic_bundles
diagnosis_reports
diagnostic_attempts
blocker_responses
work_snapshots
recovery_plans
artifact_manifests
runtime_requests
handoff_operations
merge_operations            # Phase 1，人工合并的调用意图与结果，Phase 0 不迁移

managed_resources
resource_references
cleanup_tasks
workspaces
worktree_leases
repository_dispatch_leases
gate_policy_revisions

pull_requests
pull_request_reviews
review_comments

ci_runs
ci_jobs
ci_failures

webhook_events
outbox_events
notification_deliveries

model_providers
models
model_policies

settings
```

上表是目标模型目录，不要求首版一次创建所有表。Phase 0 仅迁移实际使用的领域、执行、
交接、外部摘要、通知和配置表；Discussion、逐条 ReviewFix 和高级模型表随对应阶段启用。
待办是对 `runtime_requests`、需人工的 Requirement/运维异常和 PR 外部事实的聚合视图，不另建第二份
审批状态。通知交付记录不是业务状态真相。

`requirements` 的执行授权与当前视图字段：

```text
current_revision_id          UUID，指向唯一可执行 Contract Revision
state_version                BIGINT，领域命令 CAS 版本
generation                   BIGINT，当前执行/交接授权代次
frozen_context_input_digest  TEXT，Ready 时冻结的当前输入摘要
root_requirement_id          UUID，根 Requirement 的值为自身 ID，子任务指向根
total_recovery_attempts      INT DEFAULT 0，仅根 Requirement 维护，由 RepairAttempt 预留记录计数
gate_recovery_policy         JSONB NOT NULL DEFAULT '{"mode":"auto","allowed_failure_codes":["test_failed","lint_failed","build_failed"],"budget":3}'
failure_code                 TEXT NULL
retry_class                  TEXT NULL
manual_action_required       BOOLEAN NOT NULL DEFAULT false
```

`requirements` 不保存 Contract JSON 副本、产出提交字段或第二份 AC 副本；
Contract 及其 schema 校验只存在 `requirement_revisions.contract` /
`requirement_revisions.contract_version`，产出身份以不可变 `artifact_candidates` / `artifact_manifests` 为准；
`agent_runs.artifact_commit_sha` 只引用该 Agent 声明产生的原始候选，不被平台修复的新 SHA 覆盖。

### 14.3 关键约束

```text
requirements(project_id, repository_id, ...)
requirement_revisions(requirement_id, revision) UNIQUE
requirement_acceptance_checks(verification_batch_id, criterion_id, producer_event_id) UNIQUE
completion_declarations(agent_run_id) UNIQUE
artifact_candidates(source_type, source_id) UNIQUE
artifact_manifests(candidate_id) UNIQUE
repair_attempts(root_requirement_id, sequence) UNIQUE
repair_attempt_sources(source_type, source_id, source_version) UNIQUE
review_fix_items(repository_id, thread_id, review_version) UNIQUE
diagnostic_bundles(blocker_id, evidence_digest) UNIQUE
diagnostic_attempts(blocker_id) UNIQUE WHERE automatic = true
blocker_responses(blocker_id, idempotency_key) UNIQUE
runtime_requests(agent_run_id, worker_incarnation_id, protocol_request_id) UNIQUE
notification_deliveries(channel_id, source_event_id, notification_kind) UNIQUE
outbox_events(handoff_operation_id, action_key) UNIQUE
cleanup_tasks(resource_id, resource_generation, action) UNIQUE
outbox_events(cleanup_task_id, action_key) UNIQUE
webhook_events(provider, delivery_id) UNIQUE
worktree_leases(repository_id, branch) UNIQUE
pull_requests(provider, repository_id, number) UNIQUE
CHECK (recovery_source_type IS NULL OR recovery_source_type IN ('ci_failure', 'review_batch'))
CHECK (
  (recovery_source_type IS NULL AND recovery_source_id IS NULL)
  OR (recovery_source_type IS NOT NULL AND recovery_source_id IS NOT NULL)
)
CHECK (total_recovery_attempts >= 0)
CHECK (
  retry_class IS NULL
  OR retry_class IN ('transient', 'recoverable', 'fixable', 'blocked')
)
```

正式数据库约束必须落成以下 PostgreSQL DDL，而不只停留在应用层规则：

```sql
ALTER TABLE agent_runs
  ADD CONSTRAINT agent_runs_requirement_attempt_uq
  UNIQUE (requirement_id, attempt);

CREATE UNIQUE INDEX agent_runs_active_requirement_uq
ON agent_runs (requirement_id)
WHERE status IN (
  'created', 'preparing_workspace', 'starting_runtime', 'running', 'finishing'
);
```

普通 Requirement 的两个来源字段均为 `NULL`。Recovery 来源唯一性使用 PostgreSQL
部分唯一索引：

```sql
CREATE UNIQUE INDEX requirements_recovery_source_uq
ON requirements (recovery_source_type, recovery_source_id)
WHERE recovery_source_id IS NOT NULL;
```

从 Phase 1 起，任何自动修复先在 root 行锁内创建 RepairAttempt 并检查冻结预算；
创建子任务只是承载方式，不重复扣费。Phase 0 的一次声明后修复仍按执行预算记录。

另需约束：

- 当前 Revision、snapshot、声明、manifest、HandoffOperation 必须属于同一 Requirement；
  使用组合外键或等价事务校验，不能只验证 UUID 存在。
- candidate 的 source_type/source_id 必须引用同一需求的有效声明或 DeterministicRepair；
  修复记录必须引用原 candidate、RepairAttempt 和当前 Policy 授权。禁止循环来源、跨需求借用
  声明或覆盖旧 SHA。manifest 唯一性按 candidate，而不是 origin Run。
- 同一 Repository/branch 同时只能有一个未终结交接；同一 candidate 的同一 generation 不重复创建
  HandoffOperation。跨 generation 复用产出必须引用重验证授权（平台自动或人工）。
- gate_policy_revisions、声明、manifest、验证证据追加后不可更新；策略撤销以单独事件记录。
- 验收记录的 producer_event_id 是稳定的幂等键；新状态通过新事件追加，当前验收视图按目标
  SHA/批次派生，不保留 `(revision, criterion)` 全局唯一约束。

### 14.4 事务边界

核心事务：

| 命令 | 同一事务中的写入 |
|---|---|
| Ready/Start | 校验状态版本与 Contract、冻结 snapshot、授权 generation、领域事件 |
| 接受完成声明 | 校验当前租约/generation、插入声明、Run → Finishing、领域事件；提交后回复工具 |
| 接受 blocker | 校验当前授权、结束意图互斥、插入 BlockerReport/禁止续轮标记及探测意图；提交后回复 |
| 诊断预留 | 锁 root/全局容量并校验 blocker、输入与独立预算，写 DiagnosticAttempt/调度意图；提交后才启动 Runtime |
| 诊断结果/用户补充 | 校验 bundle/blocker 版本、来源和幂等性，追加报告或回答、刷新同一待办视图；不直接解除 blocker |
| 自动解除 blocker | CAS 核对 blocker 状态/版本、保存探测证据和恢复计划、更新待办/调度意图；不复活旧 Run |
| 输入替代 revise | 保存 pending Revision/人工授权、撤销旧 generation 和未发动作、记录保全/对账意图；完成后另事务应用 |
| 执行完成 | 确认旧组静止与声明 HEAD/工作树完整、登记已保全 candidate、Run → Succeeded、安排独立验证 |
| 验证和封存完成 | 引用已可靠保存的 sealed_ref/证据、插入 candidate 的 manifest、创建交接和 outbox；不改写 Run 状态 |
| 确定性修复完成 | 关联 RepairAttempt、不可变修复记录与新 candidate，安排新 SHA 验证；不生成 Agent 声明 |
| Cancel/输入替代 | 递增 generation/state_version、撤销待处理 runtime_requests、标记未发出命令失效、领域事件 |
| 交接成功 | 校验当前授权和远端事实、持久化 PR 关联、交接成功、Requirement → Submitted |
| 预留 RepairAttempt | 锁 root 并检查冻结上限、来源去重、预留尝试并递增计数、按需创建 Recovery 子任务、事件/outbox |
| Done | 绑定最终验证批次、输入资格及全部证据、写不可变完成快照和领域事件 |
| 受管资源释放/以上终态事件 | 更新资源引用与释放记录、upsert CleanupTask + 清理 outbox；资源未到期可先延迟入队 |

Git 对象和证据先写入平台私有的不可变存储并确认可读取，再提交引用它们的数据库事务；重启后的
孤立文件可清理，数据库不能先声明一个尚未保存的 manifest 已封存成功。
清理 outbox 使用 cleanup_task_id，交接 outbox 使用 handoff_operation_id，按事件类型校验互斥
归属；不伪造 manifest/交接字段来投递清理。清理仅处理 resource_id 所绑定的受管实例。
外部 GitHub API 调用不能放在长事务中：

```text
封存产出 → 交接/outbox → 领取一个有授权的动作 → 外部调用 → 事实归档 → 状态推进
```

每个交接 outbox 动作包含稳定 action_key、manifest_id、generation、repository/branch、目标 SHA、
预期远端 SHA、调用状态及尝试次数。Handoff Worker 的执行规则：

1. 在短事务内取得操作与 Repository lease，校验当前 generation、可交接状态、branch reservation、
   输入资格和有效验证批次的 Gate policy 未撤销，将动作标记 `Sending`；这是本次调用的授权时点。
2. push 只发送 manifest 固定 SHA。发送前 fetch 精确远端 ref：已等于目标 SHA 则归档幂等成功；
   否则必须等于 expected_remote_sha，且该 expected 必须为目标 SHA 的祖先（新分支除外）。
   祖先检查通过后，以下命令仅作为 CAS 更新守卫，不授予改写历史权限：
   `git push --force-with-lease=refs/heads/<b>:<expected_remote_sha> origin <sha>:refs/heads/<b>`，
   新建分支 expected 为 40 个 0；凭证经 `-c http.<url>.extraheader` 传 `x-access-token:<token>`，
   不进 URL、不落盘。远端与预期不符时 stderr 含 `(stale info)` → `git_push_rejected`，先对账不重试；
   其他失败按真实原因分类，只有确认的网络/5xx 等临时故障 → `git_push_failed` 可重试。远端已是目标 SHA 则 rc=0 幂等成功，但此时 git 不检查 lease，
   平台须另行 fetch 确认远端 SHA，不能把 lease 当作远端状态断言。禁止任何非 fast-forward 更新。
   远端漂移先有限只读对账，不直接转人工或启动编码。Phase 1 若 Policy 明确允许保留远端改动的
   集成修复，创建预算内 RepairAttempt，在新候选中 merge 当前远端并完整重验证，再以新预期 SHA
   交接；不能改写旧 manifest 或执行 rebase/force。无法证明输入来源、超出授权、安全策略变更、
   语义冲突或预算耗尽才转人工。Phase 0 对账确认需要集成代码时生成一次异常待办。
3. PR 创建先按 `GET /pulls?state=all&head=<owner>:<branch>&base=<default>` 查找，并校验远端 head。
   `POST /pulls` 返回 422 且 `errors[].message` 含 "A pull request already exists" 时不是失败，回到查找。
   找到已合并/关闭或不属于该交接的 PR 时转人工，不把任意同名 PR 认作成功。
4. 结果与响应丢失分开记录。push 成功但 PR 未创建、PR 已创建但响应丢失，只补当前步骤；
   重试使用相同 action_key，不创建新的 AgentRun。HTTP 权限失败不是临时重试。
5. 每个下一步骤前重新检查授权。取消/输入替代先于 Sending，则该调用不得发出；先于取消
   获得 Sending 授权的调用可能仍完成，界面显示取消处理中并保留对账状态。
6. 迟到结果只归档事实，不能让 Cancelled/NeedsRevalidation 回到 Submitted。已经出现的 branch/PR
   不自动删除；关闭 PR 等补偿动作需要用户另行授权。
7. 结果不确定、旧执行组未静止或尚有可执行旧命令时，不释放 branch reservation 给新写入者。
   对账超时转人工，不能通过过期接管假装问题消失。

交接重试等待释放短租约但保留 reservation；操作耗尽预算后 Requirement 进入 Failed
（Phase 0）或 Blocked（Phase 1+），并记录 `failure_phase=handoff`。人工 Retry/Unblock
首先尝试恢复仍有效产出的交接；只有明确选择重新编码才创建新 Run。

---

## 15. Model System

### 15.1 Phase 0 / Phase 1

Phase 0 / Phase 1 只需要支持：

```text
Runtime: Codex
Provider: configured provider
Model: configured model
Reasoning level: configured value
```

每个 AgentRun 记录实际执行配置。

### 15.2 配置层级

```text
Global Default
    ↓
Project Policy
    ↓
Repository Policy
    ↓
Requirement Override
    ↓
AgentRun Snapshot
```

### 15.3 Fallback

只对模型服务失败触发：

```text
429
timeout
provider 5xx
model unavailable
```

不把下面这些当作模型不可用：

```text
test failed
CI failed
review rejected
git conflict
```

### 15.4 预算

可以预留：

```text
project_daily_budget
repository_daily_budget
requirement_budget
agent_run_budget
```

Phase 0 即记录 Token 和估算成本，并启用并发、max_turns、执行时长、人工等待时长和阶段
重试次数上限，提供“暂停领取新任务”和“停止当前任务”。Phase 1 的有限修复另有 root 硬上限。
金额预算和准确计费集成推迟到 Phase 4，不以“尚未做预算”允许无上限执行。

---

## 16. API 设计

### 16.1 Project / Repository

```http
GET    /api/projects
POST   /api/projects
GET    /api/projects/:id
PATCH  /api/projects/:id

GET    /api/projects/:id/repositories
POST   /api/projects/:id/repositories
GET    /api/repositories/:id
PATCH  /api/repositories/:id
POST   /api/repositories/:id/validate
GET    /api/repositories/:id/gate-policies
POST   /api/repositories/:id/gate-policies
POST   /api/repositories/:id/gate-policies/:revision/approve
POST   /api/repositories/:id/gate-policies/:revision/revoke
```

Gate policy 的批准/撤销只接受人工鉴权命令，记录源 commit、配置/入口摘要、操作者和理由；
不通过 Agent 动态工具暴露。普通 Repository PATCH 不能绕过该批准流程。

### 16.2 Requirements

```http
GET    /api/requirements
POST   /api/requirements
GET    /api/requirements/:id
PATCH  /api/requirements/:id

GET    /api/requirements/:id/revisions
POST   /api/requirements/:id/revise       # 保存待应用 Revision + 明确输入替代授权，按 4.1/7.6 执行
POST   /api/requirements/:id/revalidate
POST   /api/requirements/:id/ready
POST   /api/requirements/:id/start
POST   /api/requirements/:id/retry
POST   /api/requirements/:id/cancel
POST   /api/requirements/:id/reopen
POST   /api/requirements/:id/block       # Phase 1+
POST   /api/requirements/:id/unblock     # Phase 1+，人工解除 Blocked
GET    /api/requirements/:id/acceptance-checks
POST   /api/requirements/:id/acceptance-checks  # Phase 1+，追加人工验收或豁免证据
GET    /api/requirements/:id/handoffs
```

`ready` 是评审通过动作：校验 Contract 与 AC 可验证性、冻结输入、进入 `Ready` 并立即可被领取；
对相同版本幂等。`start` 是 `ready` 的别名并额外唤醒一次扫描，仅在 Policy 开启
`require_manual_start` 时才有独立意义。两者不能绕过依赖、容量、lease 或输入资格检查。

所有写命令携带 `Idempotency-Key` 和期望 `state_version`，通过领域事务校验，冲突返回 409。
平台自动解除 blocker 和人工 Retry/Unblock 共用 RecoveryPlan 守卫，不建立不同恢复捷径。
Retry/Unblock 默认按 `failure_phase` 恢复：preparation 只重查/准备环境、不启动模型，
handoff 继续交接；validation 基础设施故障只重新验证固定 SHA，确认的代码失败按
12.5/12.6.4 预留允许的修复尝试后才启动新 Run，
execution 才在 blocker 已解除、快照与 Recovery Context 校验通过后回 Ready 创建恢复 Run。
条件未满足返回原阻塞；显式从基线重新实施须记录人工放弃中间工作的选择，撤销旧交接并完成对账。
Cancel 先撤销新动作授权；有执行组或外部调用在途时返回 202，响应包含 cleanup/reconciliation
状态，不承诺已发生的 push/PR 被撤销。Reopen 仅将 Cancelled 转 Draft，不自动启动。
人工验收必须指定 revision、criterion、subject SHA、来源证据和理由，不能修改旧证据。

### 16.3 Run

领域模型没有独立的 `Agent` 实体；UI 中的 Agent 卡片是当前 AgentRun 的视图。因此统一
使用 Run API：

```http
GET    /api/runs
GET    /api/runs/:id
GET    /api/runs/:id/events
POST   /api/runs/:id/stop
GET    /api/runs/:id/requests
POST   /api/runs/:id/approvals/:approval_id/decision
POST   /api/runs/:id/inputs/:input_request_id/response
```

审批和人工输入接口从 Phase 0 启用，按 8.1 校验请求归属、generation、状态、超时和幂等性。
`stop` 对活跃 Run 使用相应 Requirement 的取消命令，停止后不自动重试；Run 已结束则拒绝，
交接中的取消使用 Requirement API。

### 16.4 Discussion（可选）

```http
GET    /api/discussions
POST   /api/discussions
GET    /api/discussions/:id
POST   /api/discussions/:id/messages

POST   /api/discussions/:id/generate-requirements
```

ADR/OpenSpec 文件通过普通 Git 操作维护，平台不提供文档 CRUD 或审批 API。

### 16.5 GitHub / CI

```http
GET    /api/pull-requests
GET    /api/pull-requests/:id
GET    /api/ci/runs
GET    /api/ci/failures
POST   /api/pull-requests/:id/refresh        # Phase 0，节流后的只读远端同步请求
POST   /api/pull-requests/:id/merge          # Phase 1，人工合并入口；仅 Policy 关闭自动 Merge 时使用
POST   /api/pull-requests/:id/review-fixes   # Phase 1，人工补发一次 ReviewFix（自动路径不经过此接口）

POST   /webhooks/github                     # Phase 1，可选，默认不开放
```

Phase 0 的 GET 只返回最小外部事实、已校验的 GitHub URL、`last_synced_at`、`stale` 和同步错误；
`ci/failures` 首版只提供失败摘要，不承诺逐 step 诊断。`refresh` 不触发编码、修复或合并。

Phase 1 的合并有两条路径，共用同一 MergeOperation 记录与校验：

- **自动合并（默认）**：Reconciler 在 17.5 条件成立时创建 MergeOperation，`operator = platform`，
  Requirement 进入 `Merging`，以 expected head SHA 调用 GitHub。
- **人工合并（Policy 关闭自动 Merge 时）**：`POST /merge` 携带 `expected_head_sha`、
  当前 state_version、merge method 与幂等键，`operator = user`。

两条路径的服务端逻辑相同：保存调用意图，重新读取 head / 必需检查 / Review / 保护规则，
检查无活动写入者 / 修复 / 交接，绑定当前授权；不符合则拒绝。调用期间保留与执行/交接互斥的
Repository lease 和 branch reservation；未知结果未对账前不发放同分支新写入权。取消仅撤销尚未
发出的调用，已发送的请求需读远端结果。合并不绕过保护规则；已合并只启动 7.6 的最终验证，
不直接 Done。

Webhook endpoint 不提供通用写入 API，所有外部写入必须经过签名验证、去重和事件解析。

### 16.6 更新通道：首版轮询，后续可选 SSE

Phase 0 前台每 5 秒轮询待办/运行摘要，列表页可放宽到 15 秒；事件按游标分页，
不反复传输完整日志。切后台允许降频/停止，回到前台立即读取服务器状态。
显示“最后刷新时间”和断线提示，不能把连接断开显示为 Agent 已失败。
手机、电脑和平台 CLI 轮询同一平台数据库视图，不因客户端数量增加而重复调用 GitHub。

SSE 从 Phase 0b P2 作为可选增强，不阻塞完整闭环：

```http
GET /api/events
GET /api/projects/:id/events
GET /api/requirements/:id/events
GET /api/runs/:id/events/stream
```

SSE 必须支持：

- `Last-Event-ID`
- 断线重连
- 事件顺序号
- 权限过滤
- 心跳
- 慢客户端丢弃策略

### 16.7 待办、通知与控制面健康（Phase 0）

```http
GET    /api/inbox
GET    /api/notifications/status
POST   /api/notifications/test
GET    /api/system/health
GET    /api/requirements/:id/blockers
GET    /api/blockers/:id                    # ActionableBlockerView，含诊断状态与可用动作
GET    /api/blockers/:id/diagnostic-bundle   # 脱敏包，no-store、鉴权与大小限制
POST   /api/blockers/:id/actions            # action_id + 受限参数，转既有领域命令
POST   /api/blockers/:id/responses          # 回答具体缺失信息，不回答已结束 Runtime 请求
POST   /api/scheduler/pause
POST   /api/scheduler/resume
```

`inbox` 聚合未过期权限请求、待回答问题、manual_action_required=true 的失败/运维聚合项和待人工处理 PR，并返回来源实体与版本；
回答仍调用 Run API，不在 inbox 维护另一份审批。通知测试只向已批准的接收端发送脱敏测试消息，
API 不接受任意目的 URL。通知凭证通过 secret provider 配置。
blocker 写接口携带 blocker_version、期望 state_version 与 Idempotency-Key，校验归属及当前
授权。action_id 由服务器 catalog 决定并重新核查，过期操作返回 409/当前详情；不能接受任意
shell、外部 URL 或诊断模型自造动作。补充信息是审计证据，不会自动扩大权限或重置预算。
健康视图只对已认证用户开放，包含 executor、数据库、GitHub 同步、通知、磁盘和最近备份状态，
不回传凭证。暂停调度只停止新的领取，不暗中取消正在执行的 Run；停止当前任务需单独确认。

### 16.8 轻量平台 CLI（可选，0b P2）

`factory` 是拟实现的平台命令名，以下是接口设计，不是已有 Codex 命令。CLI 不是任何 Phase 的
交付门槛：正常路径上待办箱应当为空，CLI 只是处理异常项的第三个入口。

```bash
factory inbox
factory requests show <request-id>
factory requests approve <request-id>
factory requests deny <request-id>
factory requests answer <request-id>
factory watch <run-id>
```

CLI 只提供待办、请求详情、批准/拒绝、回答和运行摘要，是现有 Run API 的客户端；
不另建调度器、运行时或审批数据库。`answer` 按请求的结构收集输入，不修改原问题。
默认交互式确认，先显示仓库/Run、真实操作与范围、请求理由、过期时间，再让用户决定。
删除请求应显示明确的目标路径、是否目录/递归以及可得的变更信息，不能只显示“允许删除”；
无法界定范围时不提供笼统批准，应拒绝或要求 Agent 重新提交明确请求。

统一规则：

- 三个入口都从服务器获取相同 request_id、request_version、generation 和操作摘要；
  操作内容不可原地替换，需要变更就使旧请求失效并产生新请求。
- 决策带请求版本、操作摘要和 Idempotency-Key；沿用 Requirement 授权版本检查，
  服务端核对当前 Run、租约、有效期和状态，再原子写入唯一决定。无效请求不自动改为批准。
- 完全相同的决策重试返回已保存结果；不同决定、旧版本或过期请求拒绝。终端看到旧详情时
  不自动刷新后代用户再次批准，必须重新展示并确认。
- CLI、手机和桌面同时操作时，第一个有效决定生效，其余显示“已处理”及可见的审计来源；
  来源和身份由服务端认证确定，客户端自报 `client_kind` 不增加权限。
- 网络结果未知先读服务器决策状态，不更换幂等键重发；只允许恢复相同的已确认请求，
  不因响应丢失重新运行文件删除。区分“决定已保存”和“Runtime 已收到”。
- `watch` 先用有游标的轮询，Ctrl-C/SSH 断开只关闭观察客户端，不取消 Run 或待办。
- 首版不提供无人确认的批量批准、`approve --all` 或自动喂入 `yes`；非交互输入不能隐式批准。
  自助网页已可用时也不能以“终端输出网页链接”冒充 CLI 决策闭环。

传输与认证见 17.6。首版终端支持开发机上的人工账号以及经批准 SSH 登录后的同一命令；
不以“终端在局域网”为由免认证，不要求为原生 Codex TUI 开放公网 WebSocket。

删除文件的边界：

本节处理的是 Runtime 已发出、且平台允许人工裁决的审批请求；它不保证每次 unlink、rm、
脚本删除或 patch 删除都会自动产生一个请求。若产品将来要求“任何删除都必须预先批准”，
需另行设计覆盖所有写入路径的工具/OS 强制策略并验证，不能靠提示词或匹配 `rm` 字符串实现。
审批也不允许访问平台凭证、其他 workspace 或越过不可授权的硬边界。

### 16.9 网络授权：从"打断人"变成"需求里声明"

这是零介入路径上最常见的破口。Symphony 的实际体验是：Agent 在沙箱里需要访问外部网络，
要么停下来等人批准，要么沙箱直接拒绝导致任务失败；前者要人登录 SSH 回终端处理，
后者要重新描述需求。两种都不该发生。

原则：**一条需求的网络需求是需求的一部分，应该在评审时确定，而不是在运行时向人索取。**

#### 三层来源，从宽到严

```text
1. 需求级声明（Requirement Contract.network_access）
   评审时由用户或 Contract 草稿生成器写出，随 revision 冻结。
2. Repository 默认（Repository Policy.network_defaults）
   该仓库常见依赖域名、包管理器、CI 端点的默认集合。
3. 平台全局白名单（对需求只读；管理员可版本化批准/撤销）
   provider 端点、必要的官方 registry；不能改写全局配置，需求 denied 仍可缩小自身有效集合。
```

生效集合 = 平台全局 ∪ Repository 默认 ∪ 需求声明；`denied_domains` 优先级最高，谁都不能覆盖。
需求声明超出 Repository / 平台允许范围时，评审阶段（`POST /ready`）就拒绝并说明缺哪个域名，
而不是等 Agent 跑到一半才失败。

#### 声明形态

```toml
[requirement.network]
# 语义化预设优先，评审者不必手写域名
presets = ["cargo", "npm", "github_api", "cloudflare_api"]

# 预设之外的显式域名（自动去重、自动展开子域）
domains = ["download.pytorch.org", "huggingface.co"]

# 明确禁止（覆盖一切上层）；空表示不额外禁止
denied = []
```

预设是平台维护的命名集合（例如 `cargo` = `crates.io` / `static.crates.io` / `index.crates.io` /
`github.com` / `raw.githubusercontent.com`），Repository Policy 可以扩充但不能再定义同名预设。

#### 运行时执行：Codex 的受限网络模式（S5 实测，见 23.S）

Codex 0.153.4 自带一个可用的机制，不需要平台自己写代理：

- 开关：`features.network_proxy = true`；种子环境时 `[network] enabled = true`。
- 沙箱任务获得一个**本地 CONNECT 代理**（`http_proxy` / `https_proxy` 指向 `127.0.0.1` 的随机端口），
  非白名单域名的 TLS 连接被代理层拒绝：
  `CONNECT tunnel failed, response 403 ... was blocked: domain is not on the allowlist for the current sandbox mode`。
- 这是**真正的域名白名单**：`approvalPolicy = "never"` 下也不会请求批准，直接拒绝并返回明确错误文本给 Agent。
- 直连绕过（`--noproxy '*'` 打 IP）被沙箱网络隔离挡住；白名单生效时绕过路径也一并关闭。

平台侧要做的是：按 16.9 的生效集合生成 allowlist，并把上游代理（用户网络需要时）通过进程环境传给
app-server。**S5 实测：allowlist 只能写在系统级 `/etc/codex/requirements.toml` 的
`[experimental_network]` 表下（需 root），用户 `config.toml` 与 `$CODEX_HOME/requirements.toml`
都不生效。** 这意味着该配置是全局的、影响同机所有 codex 进程，不是 per-Run 的。

由此产生一条实现约束（16.9 的核心难点）：

- 平台必须把 `[experimental_network]` 当作**独占的受信资源**管理：由受信控制面写、Agent 不可写；
- 每个 Run 开始前写入该 Run 的生效集合（并记录摘要用于审计），Run 结束后收敛；
- **同机不能并行跑两个网络集合不同的 Run**——这与 6.3 的"同 Repository 串行""全局并发上限"
  是不同维度的约束，必须在 Phase 0 的调度条件里显式表达，不能靠"反正并发是 1"隐式成立；
- 多 Worker（Phase 4）时每个 Worker 主机各自持有自己的 `/etc/codex/requirements.toml`，
  调度器必须按网络集合对 Run 做**主机亲和性**分组，或串行化不同集合的 Run。
- 从 0a 起保存该主机网络策略摘要、持有者与恢复状态；文件修改由预先配置的最小特权 helper
  执行，仅接受已批准集合，不以 root 启动整个 app/Agent。部署必须明确授权该受管配置。
  无法排除同机其他 Codex 使用者时，禁止按 Run 改写全局文件；选择已批准的固定集合或
  隔离 executor 的配置视图。Phase 2 并发仅允许共享同一有效集合，不能静默取不同需求的并集
  扩大授权；最后使用者退出才收敛。重启先对账，不覆盖未知使用者的配置。

#### 与需求契约的关系

- `RequirementContract` 增加可选字段 `network_access {presets[], domains[], denied[]}`；
  缺省表示只使用 Repository 默认，不允许用"未声明"表达"全放开"。
- Agent Context 中包含生效集合的摘要，让 Agent 知道哪些域名可用，减少无效尝试。
- 单次未声明域名请求被拒时，Agent 可继续其他工作，不产生权限请求或待办。工具拒绝记录
  不等于 Run 失败。需要补充范围时通过 report_blocker 的结构化证据提交 network_scope_request
  建议；该结束工具只在确实无法继续时调用，不用于仍可继续任务的普通诊断。
- 确实无法继续时必须报告 blocker、停止旧组并保存工作；已有授权内的固定依赖准备按 8.1
  自动恢复。需要扩大范围时按 requirement + 建议集合去重，生成一次 Contract 修订待办，
  manual_action_required=true；不消耗代码修复预算，不向 Runtime 发送临时网络批准。
- 用户采纳时显示新增域名、原因和恢复阶段，保存新 Revision 并记录授权；随后自动预检和恢复。
  Phase 0a/0b 按下面的受控输入替代路径撤销旧 generation、保全工作、回 Ready；0c+ 使用
  NeedsRevalidation。不能在原 session 中静默扩大 allowlist。

#### 验收

```text
已声明 cargo 域名 → 放行，不产生 runtime_request；
单次未声明域名拒绝但任务可继续 → 无待办，不结束 Run；
已授权依赖暂缺 → 固定准备/有限探测，满足后自动恢复，无需 Unblock；
无法继续且需新域名 → 一次范围修订待办，保存快照，重复报告不新增通知；
用户采纳 → 新 Revision/授权、预检和从保存步骤恢复，不复用旧 session。
```



#### 不在本节范围

- 平台不实现通用出网代理、DNS 重绑定防护或流量内容审查。
- 非 Codex Runtime（后续 Phase）需要各自的等价机制；网络策略属于 Runtime 能力契约的一部分，
  新 Runtime 接入时必须先满足 16.9 的拒绝语义。
- 网络白名单不替代凭证隔离：白名单里的域名不等于允许携带平台凭证。

---

## 17. 安全设计

### 17.1 Agent 不可信输入

以下内容一律视为不可信输入：

- Requirement 描述
- Discussion 内容
- ADR/OpenSpec 文档
- Repository 文件
- PR 评论
- CI 日志
- Webhook payload

Agent Context 必须包含来源和边界信息，不能默认相信其中的指令。

### 17.2 Workspace 隔离

必须：

- canonical path containment
- workspace root 权限隔离
- 独立 worktree
- 进程组级别终止
- 禁止操作其他 workspace
- 限制 hook 工作目录

上述路径规则不替代操作系统隔离。Phase 0a/0b 仅限可信自有仓库，使用同 UID 的已知读取
限制，仍必须控制写入、子进程停止和凭证不注入；不宣称具备下述完整读取隔离。
从 Phase 0c 起，Agent 工具、after_create/before_run/after_run、
依赖安装脚本以及 Gate/test 子进程都在低权限执行环境运行，不继承 API/调度器的执行身份。
必须限制可读挂载和可写目录、网络出口、进程/CPU/内存额度；禁止挂载 Docker socket、
平台数据库凭证、GitHub App 私钥、Cloudflare Tunnel 凭证、通知密钥和 evidence store 的写权限。

受信控制进程负责判定和归档，仓库代码只提供不可信输出。执行组以 cgroup、容器或等价可验证
的 OS 边界标识并完整终止，不能仅按 PID 杀单进程。生产就绪检查必须实际验证凭证不可读、
其他 workspace 不可访问、子进程不可逃逸；环境不具备这些能力时拒绝启动写入型 Run。

### 17.3 凭证

凭证只允许存在于：

```text
GitHub App credential provider
Model secret provider
Runtime host environment
Cloudflare connector secret store（仅 cloudflared 可读）
Notification credential provider（仅通知发送器可读）
```

不得：

- 写入 Requirement
- 写入 Agent Context
- 写入日志
- 写入 WORKFLOW.md
- 通过普通环境变量直接继承到 Agent

S1 实测：workspace-write 沙箱只限制写，Agent 的 shell 子进程可以直接读取 `~/.codex/auth.json`
和 `~/.ssh/`。因此 Phase 0a 的单进程形态等价于"Agent 拥有平台运行用户的全部读权限"，
只能接管自己可信的仓库；0c 的独立 UID executor 不是可选加固。
Codex app-server 的模型认证与工具子进程的可见环境分开：只读挂载认证文件本身并不能阻止
读取，必须验证工具执行视图无法访问该文件及父进程凭证；若锁定版本/宿主隔离不能满足，
使用受控认证代理，或拒绝该部署模式。GitHub 凭证只由交接控制端持有，LocalGitBroker、
hooks 和验证步骤均不持有。日志脱敏不是凭证隔离的替代品。

### 17.4 API 和 Webhook

Phase 0 使用 Cloudflare Access 保护整个管理域名，浏览器不保存长期静态 Bearer Token：

- Access 只允许明确的个人身份，不以“拥有域名”或“能打开 Tunnel”视为登录成功。
- API 校验 `Cf-Access-Jwt-Assertion`（S3 实测规则，见 23.S）：`alg` 白名单仅 RS256；
  `iss == https://<team>.cloudflareaccess.com`（部署时固定）；`aud` 数组包含固定的应用 AUD；
  `exp` / `nbf` 带 ≤30 秒 leeway；**`sub` 是 Access 的稳定身份 ID（电脑与手机相同），作为平台
  `owner_id` 的绑定值**；`email` 与允许名单二次比对；`type == "app"` 可选校验。
  `Cf-Access-Authenticated-User-Email` 头只用于显示，不作为身份凭据。JWKS 来源为
  `https://<team>/cdn-cgi/access/certs`，按 `kid` 缓存，未知 kid 限频重取一次；
  不能从未校验的 JWT 接受任意 issuer/JWKS 地址。拒绝原因按子码
  `missing_assertion / unknown_kid / bad_alg / invalid_token / email_not_allowed` 记录。
- 首版浏览器直接使用 Access 会话，不另建账号密码、设备配对或双重登录系统；会话期建议配置为
  8 小时。到期要求重新登录，平台待办不丢失，写请求不在登录后自动重放。
- cookie 登录仍需 CSRF 防护：写 API 检查 Origin、仅接受预期 JSON 类型和自定义请求头，
  使用同源部署和严格 CORS；GET 不产生审批/合并等副作用。
- 管理 API 设置请求体/速率限制、日志脱敏；敏感 API 响应 `Cache-Control: no-store`，
  不通过边缘缓存或未来 service worker 缓存审批、日志、凭证和私人数据。
- 仅允许 Tunnel 连接器访问 Web 源站入口；Agent 网络不可访问控制面，源站不开放绕过 Access
  的 LAN/公网端口。内部健康检查使用独立受限通道，不作为管理 API 鉴权后门。
- 丢失手机时先撤销 Access 会话/身份授权（应用的 Revoke existing tokens，S3 实测手机与电脑刷新
  即被踢回登录页），必要时在源站禁用该用户；不把清除浏览器 cookie 当作服务端撤销。
  恢复访问通过宿主机管理通道执行。
- **撤销只在边缘生效**：源站看到的 JWT 是无状态的，已签发 JWT 在 `exp` 前直接送到源站仍会验签
  通过。因此"源站只能经 Tunnel 到达"是撤销语义成立的前提；会话时长不得超过 8 小时。
  若将来源站必须在 LAN 上监听（连接器在另一台主机），源站须按 `identity_nonce` 维护短期
  撤销名单或把会话缩到 ≤1 小时。

Phase 0 不开放 Webhook；Phase 1 如启用，另用精确路由执行 GitHub HMAC、原始请求体校验、
delivery 去重和重放防护。该入口只能写入 webhook inbox，不能访问管理命令。
若其路径配置 Access bypass，范围仅限该回调路径，绝不能豁免整个管理域名。
Cloudflare Access 与 webhook HMAC 是两套独立信任边界，不能用其中一套替代另一套。

### 17.5 自动 Merge

Phase 0 用户在 GitHub 合并。Phase 1 起自动 Merge 默认开启；Repository Policy 设置
`auto_merge = false` 可进入信任建立期，此时 16.5 的人工合并入口生效，其余流程不变。

自动 Merge 的条件全部由平台从外部事实和验证证据机械判定，不含任何主观项：

```text
PR head 的必需 Checks 全部 success（Policy 按 job `name` 声明集合，如 `test-job`；
          按发布 App、workflow/job、触发事件和 Policy selector 分组，每个必需组的当前有效 attempt
          必须 success；push 与 pull_request 是独立组。旧失败 attempt 保留历史，不永久阻止新成功；
          缺失 / skipped / neutral 不算通过；不用 combined status——Actions 仓库下它恒为 pending）
AND 受信 Gate 在该 head 上通过
AND `pulls/:n.mergeable_state == "clean"`（GitHub 异步计算，首次读取可为 null，Reconciler 重试；不需要 administration 权限）
AND 所有 review thread 已 resolved（无 reviewer 的仓库此项恒真）
AND Policy 要求的 approval 已存在（默认不要求；单人仓库不能靠 Agent 伪造第二人批准）
AND 所有 pre_merge 适用且未被明确豁免的 AC 在该 head 上 verified
AND 无进行中的 RepairAttempt / Agent 写入 / 验证封存 / Handoff（不以子 Requirement 未 Done 判定）
AND Requirement 无未解决的 NeedsRevalidation
```

安全边界：

- 平台只申请 `pull_requests: write`，不加入 branch protection bypass 列表；保护规则由 GitHub 强制，
  平台不能也不应绕过。
- 合并方法由 Policy 固定（默认 squash），commit message 由平台生成并附 Requirement 追溯标记。
- 合并后立即建立 `post_merge` 验证批次（7.6）；验证失败不回滚合并，而是自动创建
  `revert` 类型的 follow-up Draft 进待办箱，由人决定是 revert 还是 forward-fix。
- Agent 不能触发合并、不能修改合并条件、不能修改 Policy。

信任建立期建议：前 N 条需求（N 由用户自定，例如 10）关闭自动 Merge，人在 GitHub 看 PR 再合；
观察到 AC 验证与自己判断一致后打开。

### 17.6 终端认证与控制面隔离（可选 CLI 适用）

本节仅在实现 16.8 的可选 CLI 时适用。平台 CLI 使用开发机上独立的受限 Unix socket 访问审批 API；不通过公开 HTTP 的免鉴权
例外，不从浏览器复制 Access cookie，也不要求原生 Codex TUI 能完成 Access 登录。
浏览器走 Access 身份验证；本地 CLI 走操作系统身份验证，两者映射到同一平台 owner_id，
再进入同一个授权/CAS/审计实现，不是两套审批规则。

- socket 只提供 16.8 所需的待办/Run 读取和审批/回答命令，不能启动任意进程、写 SQL、
  改策略或转发任意 app-server RPC；复用 API 的校验、大小/速率限制。
- socket 所在目录与文件只授权指定人工管理账号；服务端验证 OS peer credentials，
  将配置允许的宿主机 UID 映射到平台 owner_id，拒绝其他 UID。容器场景必须验证实际 UID
  映射，不因 socket 能连上就默认可信，也不相信客户端发送的 user_id。
- Agent/executor 使用不同的低权限 UID，不挂载该 socket 或其父目录，不得进入人工账号
  会话，也不能读取 SSH 私钥。若无法满足隔离条件，终端审批模式不得启用。
- 远程电脑的终端首版可通过已批准的 SSH 连接到该开发机后运行平台 CLI；
  SSH 主机身份/登录仍按原有安全策略校验，不新增公开终端、SSH 入站端口或 root 登录要求。
  实现该功能不授权平台擅自配置路由器、Tunnel SSH 或用户的 SSH 密钥。
- 断开 SSH、退出 CLI、停止 watch 都不影响后台审批请求；禁用人工 UID 或移除 socket
  授权后，后续请求拒绝。宿主机 root 属于主机管理员信任边界，不宣称能防御主机被完全接管。
- 从其他机器直接以 HTTPS 运行 CLI 的认证方式留作后续增强；如接入 Access service token，
  需要单独定义作用域、撤销和人类决策审计，不能把自动化身份默认为人工批准。

---

## 18. 本项目 Angular 视觉与交互规范

本项目自身的 Angular 前端参考：

```text
arc-admin/docs/ui-design-system.md
arc-admin/frontend/src/styles/_tokens.scss
arc-admin/frontend/src/styles.scss
```

这套规范只约束 Personal AI Software Factory 的前端，不作为外部 Repository 的 UI 约束。
所有新页面、改版页面和移动端适配都必须遵循同一套语义 Token、页面模式和视觉验收。

### 18.1 设计语言

使用 Angular Material M3 组件能力，并采用 arc-admin 的 Arco-inspired 克制蓝视觉：

```text
字体：系统无衬线字体，中文优先 Noto Sans SC
主色：语义 primary，页面 SCSS 不写业务十六进制颜色
密度：紧凑但可触控
表面：浅色 / 深色双主题
状态：文字 + 颜色 + 图标或结构共同表达
```

样式来源优先级：

```text
styles/_tokens.scss
    → 字体、间距、圆角、控件高度、颜色、阴影、动效 Token
styles.scss
    → 两个及以上页面复用的布局和组件模式
page.scss
    → 单页面特有布局和列宽
Angular Material
    → 通过 --mat-sys-* 语义变量适配
```

新增颜色、尺寸或动效前，必须先搜索已有 `--ui-*` Token。不得通过高 specificity、
`!important` 或深层选择器覆盖 Material 内部实现。

### 18.2 Token 基线

间距使用 4px 阶梯：

```text
4 / 8 / 12 / 16 / 20 / 24 / 32 / 48 / 64px
```

控件高度：

```text
small = 32px
medium = 40px
large = 44px
```

圆角：

```text
2 / 4 / 6 / 8px
pill = 9999px
```

动效使用 duration/easing Token，禁止 `transition: all`。焦点轮廓必须可见，并在
`prefers-reduced-motion: reduce` 下关闭非必要动画。

### 18.3 标准页面骨架

每个页面只有一个可感知的 `h1`，应用壳层只提供一个 `main` landmark：

```html
<section class="page">
  <header class="page-header">
    <div>
      <h1>页面标题</h1>
      <p>一句话说明页面用途</p>
    </div>
    <div class="header-actions">主要操作</div>
  </header>

  <div class="filter-bar">搜索、筛选和结果数</div>
  <div class="table-card">表格、加载、错误或空状态</div>
  <section class="stats-grid">可选统计摘要</section>
</section>
```

页面最大宽度 1200px，桌面内边距 24px，窄屏收紧为 16px。主要操作位于右侧
`.header-actions`，每个区域通常只保留一个 primary action。

跨页面复用以下模式，不复制出视觉相同但维护分叉的实现：

```text
.page
.page-header
.header-actions
.filter-bar
.table-card
.table-scroll
.data-table
.status-badge
.loading-wrap
.empty-state
.stats-grid
.stat-card
```

### 18.4 本项目页面模式

首页是待办箱 `Needs My Attention`，设计目标是**大多数时候为空**。顶部只放需要人决策的项，
按紧急度排序；下方是只读的进度概览：

```text
需要我处理（默认路径上应为空）
  待回答的 Agent 提问
  待批准的沙箱外请求（仅 Policy 设为 ask 的类别）
  Blocked / 修复预算耗尽
  安全类门禁失败
  自动重验证失败的 NeedsRevalidation
  post_merge 验证失败的 revert 建议
  待审查 PR（仅 Policy 关闭自动 Merge 时出现）

进度概览（只读）
  Running Agents
  Ready / Queued
  Submitted → WaitingCI → WaitingReview → Merging
  最近 Done
  Recent Activity
```

待办箱连续为空是系统健康的信号，不是异常；UI 用空状态明确显示"没有需要你处理的事"，
并附最近一次自动完成的 Requirement。

Phase 0 的 CI 失败和已合并状态来自只读同步；Phase 1 增加持久化 Blocked、自动验收结果和修复队列。
统计卡片和全量审计表不抢占手机首页空间。

首版只要求四类页面：待办箱/进度概览、需求新建与评审、Run 进度与异常处理、PR/CI 摘要。

阻塞卡片从 0a 起是强制产品契约，不能仅展示 Blocked、failure_code 或日志链接：

| 用户必须能回答的问题 | 卡片必须展示的内容 |
|---|---|
| 为什么停了？ | 失败阶段/具体步骤、一句话原因与 confirmed/suspected/unknown；关键脱敏证据可展开 |
| 已经做到哪里？ | 已完成与尚未验证的工作分开，快照是否完整、哪些工作仍保留 |
| 系统正在做什么？ | 已尝试动作及结果、当前诊断/恢复动作、下次检查时间；若不再自动处理则说明原因 |
| 我现在需要做什么？ | 明确“无需操作”或具体执行主体、对象/范围、步骤与动作按钮；禁止含糊的“修复环境” |
| 怎么知道解决了？ | 解除判据、平台会执行的检查、上次检查结果；不能用用户点击代替验证 |
| 然后从哪继续？ | 下一恢复阶段与保留工作，不显示泛化的“重新运行任务” |

字段暂缺时显示“尚未确认/正在采集/采集失败原因”，不得留空或伪造根因。诊断进度、
等待容量、证据缺失和失败降级都在同一详情页；无需用户到另一个 Agent 会话查日志。
通知从 0b 起给出脱敏的一句话原因、是否需要操作及详情链接，不只写“任务被阻塞”。
普通诊断进展不重复通知；从自动处理转为需人工时发送一次，按 blocker/version/通知类型去重。

示例（说明模板，不代表实际故障）：

```text
测试依赖下载受阻｜原因已确认｜需要你允许本需求访问一个新域名
失败位置：准备测试依赖；代理拒绝 download.example.org
已完成：代码修改已保存；完整工作快照可用；尚未完成测试
系统已尝试：本地依赖与已批准镜像检查，均无可用版本
你需要做：查看新增域名与原因，选择“采纳本需求网络修订”或“保持当前范围”
解除判据：新授权生效、沙箱内下载探针成功、依赖摘要一致
之后继续：从依赖准备与测试继续，不从头编写代码
```

未知原因示例必须显示“固定诊断未确认原因”，给出已排除项、缺失证据与具体采集步骤；
“重新检查”只执行已授权固定探针，“采纳修订”进入 4.1/16.9 的版本化授权，不是通用提权按钮。

日志、证据、失败原因作为详情展开，不另建完整后台模块。

审批卡片必须在手机可见范围内显示：

- Requirement、Repository、当前 Run 和请求来源；
- 实际命令/工具参数、工作目录、读写路径或网络目标；敏感值脱敏，但不能隐藏操作范围；
- 请求理由、权限变化与风险提示；风险提示只是辅助，不能覆盖硬性禁止规则；
- 过期时间和当前状态；单次批准、拒绝及理由输入；
- 明确区分“决策已保存”“Runtime 已接收”“请求已失效”，多设备同时操作时显示最终结果。

人工问题提供直接回复输入框；已处理卡片保留结果但禁用再次提交。通知打开的是这个
可重新鉴权的详情页，不是带有批准令牌的 action URL。

Requirement、Agent、PR、CI 详情页统一采用：

```text
上下文标题
状态摘要
主要操作
Project / Repository / Branch 关联
事件时间线
Harness-Gate / CI 证据
```

Agent 事件至少区分：

```text
Workspace
Runtime
Turn
Tool Call
Gate
Commit
Push
PR
CI
Review
```

门禁失败必须显示稳定 failure code、失败步骤、简短原因和报告入口，不能只显示泛化错误。

### 18.5 表格和异步状态

语义表格要求：

- 使用真实 `<table>` 和唯一 `<caption>`；
- 表头使用 `scope="col"`；
- 行操作列置于末尾；
- 加载时设置 `aria-busy`；
- 加载、空和错误状态互斥；
- 状态不只依赖颜色，必须有文字；
- 动态长文案不能用固定高度强行裁切。

每个异步页面必须覆盖：

```text
Loading
Success
Empty
Failure + Retry
```

提交中按钮必须禁用并显示动作状态；成功反馈使用 `role="status"`，错误使用
`role="alert"`。危险操作必须说明对象和影响，并要求明确确认。

### 18.6 响应式和移动端

断点基线：

```text
≤ 1024px：收窄侧栏和辅助导航
≤ 768px：切换移动导航
```

普通表格在自身容器内横向滚动，不让页面整体溢出；审计、事件和长文本在窄屏切换为卡片
列表。筛选栏在窄屏纵向排列，输入和选择器占满可用宽度。

本项目页面至少在以下视口验证：

```text
Desktop Chrome
Pixel 7 / mobile Chromium
```

响应式手机 Web 属于 Phase 0 核心能力，不以“PWA 在 Phase 4”为由推迟。完整安装体验、
service worker、Web Push 和离线模式可后置；首版不承诺离线执行或后台轮询可靠。
除模拟视口外，至少在用户实际使用的手机浏览器验证 Access 登录、通知跳转、审批、
输入法弹出、锁屏返回和 GitHub 跳转；若使用 iPhone，须补 Safari 真机流程，
不能用 mobile Chromium 模拟替代。

### 18.7 无障碍

必须支持：

- Skip link 和唯一 main landmark；
- 图标按钮、菜单、复制、主题切换的准确可访问名称；
- 菜单 `aria-expanded` 和关闭后焦点回归；
- 表单错误与控件程序化关联；
- 键盘可见焦点；
- 亮色、暗色、强制颜色模式可读；
- `prefers-reduced-motion`；
- 对话框关闭后焦点返回触发控件。

### 18.8 本项目视觉验收

本项目至少提供以下等价的前端检查，并通过 Harness-Gate 编排：

```bash
npm run lint
npm run format:check
npm run test:ci
npm run build
VISUAL_REVIEW=1 npm run e2e -- --project=chromium --project=mobile-chromium
```

视觉复核必须检查标题/操作/筛选/表格顺序、页面横向溢出、加载/空/失败/禁用状态、
键盘焦点、对话框尺寸、Snackbar 位置、亮暗主题、减少动效、表格语义和 Token 复用。

## 19. Observability

### 19.1 结构化日志

每条相关日志尽量包含：

```text
project_id
repository_id
requirement_id
agent_run_id
session_id
workspace_id
event
outcome
duration
error
```

### 19.2 Agent Event

事件分为：

```text
agent_started
workspace_created
runtime_started
session_started
turn_started
tool_call_started
tool_call_completed
approval_required
turn_completed
agent_failed
agent_stopped
blocker_reported
blocker_resolved
work_snapshot_saved
work_snapshot_failed
recovery_context_degraded
recovery_run_started
```

事件必须限制 payload 大小，原始敏感内容不能无上限落库。

### 19.3 指标

Phase 0 / Phase 1：

```text
running_agents
ready_requirements
submitted_requirements
retry_count
agent_duration
token_usage
estimated_cost
```

Phase 0a 起按 Requirement/Run/阶段持久化 12.7 的效率计数，可观察的模型请求按 request
身份去重，恢复和重启不得覆盖累计值。Runtime 只暴露 turn 时另记 turn_count，不能假定
一个 turn 等于一个底层模型请求；不可观察的 model_call_count 标记 unknown：

```text
model_call_count             含编码、修复、只读诊断和可选摘要调用，分别标记用途
input_tokens / cached_input_tokens / output_tokens
uncached_input_tokens        仅在提供方明确缓存口径时由 input - cached 得出，否则 unknown
phase_duration              preparation / execution / validation / ci_wait / handoff / human_wait
preflight_failure_count / preparation_attempts
validation_execution_count / validation_duplicate_suppressed_count
repair_context_bytes / summary_model_call_count
blocker_count / recovery_run_count / recovery_context_degraded_count / work_snapshot_failure_count
blocker_reason_confirmed/suspected/unknown_count / actionable_blocker_missing_field_count
blocker_time_to_explanation / blocker_time_to_action / blocker_resolution_duration
diagnostic_bundle_incomplete_count / fixed_diagnosis_duration
diagnostic_attempt_count / diagnostic_tokens / diagnostic_turns / diagnostic_duration
diagnostic_invalid_output_count / diagnostic_budget_exhausted_count / manual_diagnosis_required_count
```

Phase 1 增加 evidence_reuse_count、evidence_reuse_rejected_count、deterministic_fix_count、
repeated_failure_stop_count。证据复用不等于模型调用节省，分别计量；缺失的 token/cache 数据
不得填零。金额只按实际提供方计费口径估算，不将含缓存和重复上下文的 total_tokens 直接估价。
业务决策事件独立于高频 delta 调试流保存，调试日志限额/轮转不得挤掉恢复和验收证据。
8.5 增加 observation_original/returned_bytes、observation_recall_count/bytes、archive_failure_count；
12.7.6 实验增加 fusion_eligible/triggered、实际请求变化、重复/无效检查数。8.6 启用时记录
compaction_reason、摘要与缓存重建成本、压缩后额外请求及预计/实际收益。估算与实测分别标注。
0a 同时记录 managed_resource_count、cleanup_pending/failed/deferred、最老可执行清理任务等待
时长、cleanup_duration、实际 reclaimed_bytes、补漏任务数；retention 未到与清理积压分开显示。
磁盘/资源视图能定位 owner Run、保留原因、最后错误及下一重试时间。清理临时失败不通知；
永久失败/重试耗尽、可执行积压超过 Policy 阈值（默认 30 分钟）或影响调度时聚合运维待办，
Deferred 的引用/保留期等待不算积压。

Phase 0 增加 pending_approvals、pending_inputs、notification_delivery_failures、
github_sync_age、ci_failure_count；Phase 1 增加 queued_requirements、blocked_requirements、
review_fix_count、auto_merge_count、post_merge_validation_failures。CI 摘要计数不等于内部状态计数。

零介入是产品目标，因此从 Phase 0a 起单独维护介入指标，按 Requirement 归档、按周聚合展示：

```text
统计口径：固定同一批 Ready 授权的根 Requirement（相同观察窗口），以该批全部需求为分母。
分子仅包括零人工介入且达到该 Phase 成功终点的需求（0a 为通过本地适用 AC 后 Submitted，
Phase 1 为 Done）；失败、取消和未完成均保留在分母并单列，不因重试/子任务增加样本量。
恢复、输入修订与重新授权不覆盖原累计；Policy 人工模式另分组，但不静默排除失败样本。

intervention_count            Ready 后人工处理次数，包括输入替代/扩大授权；自动探测/清理不计
intervention_reason           agent_question | sandbox_ask | network_scope_change | security_gate |
                              budget_exhausted | environment_manual | recovery_exhausted |
                              storage_recovery | handoff_manual | revalidation_failed |
                              input_revision | post_merge_revert | reviewer_wait | operations | manual_policy
zero_touch_ratio              零介入且验收成功数 / 同批全部根需求数（0a ≥60%，Phase 1 ≥70%）
time_to_pr / time_to_done      从 Ready 起算的实际端到端时长，包含人工等待
human_wait_duration           人工等待单列
active_processing_duration    另报扣除人工等待的处理时长，不冒充端到端时长
```



`intervention_reason` 不在上述枚举内的介入视为缺陷；`agent_question` 占比高说明 Contract 模板需要改，
`sandbox_ask` 占比高说明网络白名单初值需要调。

V2：

```text
OpenTelemetry
Prometheus
Grafana
```

### 19.4 Dashboard

首页不是传统 Admin Dashboard，而是工作队列：

```text
Needs My Attention
├── 待审批 / 待回答
├── PR 待审查 / CI 失败
├── 执行失败 / 同步或通知异常
├── Running Agents（包含人工等待原因）
├── Ready Requirements
├── Submitted Requirements
└── Recent Activity

Phase 1：
├── Queued Requirements
├── Blocked Items
└── 最终验收 / 修复队列
```

MVP 展示已接入的 PR/CI/Review 外部摘要和同步时间，不展示尚未启用的持久化 Blocked/Done
或自动修复统计。手机首页以待办和明确下一步操作为主，不要求监控大屏。

必须支持按以下维度过滤：

```text
Project
Repository
Requirement State
Agent State
PR State
CI State
```

### 19.5 首版通知

首次部署选定并只实现一个用户实际接收的渠道（例如现有聊天工具机器人或受控通知服务）；
渠道选择是部署配置，不需要首版建设通知平台，也不假定已有任何厂商凭证。
真实手机收到一次通知是交付条件，不能用“未来支持”或仅日志输出替代。

- 通知事件：需人工的新审批/问题、自动恢复耗尽、PR 已可人工处理、授权扩大和运维告警，
  以及请求即将过期的单次提醒。自动 CI 修复、环境探测和清理重试只更新进度，不逐次通知。
- 仅发送脱敏标题、事件类型、时间和固定站点下的详情链接；不发送原始命令、代码、secret、
  Access token 或批准口令，锁屏通知不暴露仓库敏感内容。
- 事件与 outbox 在同一事务写入，发送结果保存到 `notification_deliveries`；同一来源事件
  重复消费不重新生成通知意图。发送器不要求恰好一次送达：响应丢失可能重复提醒，但绝不能
  重复批准或触发编码。重复轮询得到同一事实不产生新事件。
- 发送失败有限退避重试，超过通知时限显示失败/过期；已解决请求不继续发送过期催办。
  通知故障不导致 Run 失败，不占用执行重试预算，不使审批自动通过。
- 通知是提醒渠道，数据库待办才是权威；即使通知不可用，用户仍可打开平台处理。
  浏览器关闭后不依赖前端轮询发送通知；通知发送始终由开发机后端负责。
- 本机断电时本机不能发送告警；独立外部可用性监控可后置，首版必须明确显示服务不可达，
  不承诺断电期间仍能审批或执行。

---

## 20. Rust Workspace 建议

Phase 0 / Phase 1 不要把系统拆成过多独立服务，但可以保留清晰 crate 边界：

```text
personal-ai-factory/
├── apps/
│   └── server/
├── crates/
│   ├── domain/
│   ├── persistence/
│   ├── scheduler/
│   ├── agent-runtime/
│   ├── workspace/
│   ├── github/
│   ├── ci/
│   └── recovery/
├── web/
│   └── angular/
├── migrations/
├── docs/
├── docker-compose.yml
└── Cargo.toml
```

推荐技术：

```text
Rust
Tokio
Axum
SQLx
PostgreSQL
reqwest
serde
tracing
thiserror
uuid
chrono
async-trait
```

### 20.1 domain

只放：

- Project
- Repository
- Requirement
- AgentRun
- Domain Events
- State transitions

不依赖 Axum、SQLx 和 GitHub SDK。

### 20.2 scheduler

负责：

- queue
- priority
- dependency
- global capacity（Phase 0 / Phase 1 固定 1；Phase 2 默认 1、可配置到 4）
- repository serial（Phase 0 / Phase 1 同 Repository 串行）
- project/repository capacity（Phase 3）
- fair scheduling（Phase 3）
- claim lease
- retry

### 20.3 agent-runtime

负责：

- Codex protocol
- runtime adapter
- session lifecycle
- tool calls
- approval handling

### 20.4 persistence

负责：

- repository
- transaction
- outbox/inbox
- migrations
- optimistic locking

### 20.5 github

负责：

- GitHub App
- repository API
- PR
- review
- checks
- webhook parsing

### 20.6 已由 spike 确定的依赖

| crate | 版本 / feature | 来源 |
|---|---|---|
| `jsonwebtoken` | `11`，**必须** `features = ["rust_crypto"]`（或 `aws_lc_rs`）；缺省 feature 编译通过但首次验签 panic | S3 |
| `reqwest` | `0.12`，`default-features = false, features = ["json", "rustls-tls"]` | S3 |
| `axum` | `0.8` | S3 |
| App JWT 签发 | RS256，`iat` 提前 60s、`exp` ≤ 10 min、`iss = app_id`；同一 `jsonwebtoken` 可签 | S2 |
| Codex 协议类型 | `codex app-server generate-json-schema --experimental --out <dir>` 产物 codegen（0.153.4 共 47 个文件） | S1 |

---

## 21. 部署

### 21.1 Phase 0 / Phase 1 Docker Compose

```text
Phase 0a（本机开发态）
├── app + executor 合一：一个 Rust 进程直接 spawn codex app-server 与 git，
│   跑在开发者自己的 UID 下；只接管自己可信的仓库
└── postgres（docker 或本机）

Phase 0b 起
├── app                  # Web/API、调度、可信控制面、只读同步、通知发送
└── postgres
既有 cloudflared 连接器 → app 的受限入口
（没有既有连接器时才新增 cloudflared 服务，不重复搭建第二套）

Phase 0c 起
├── app
├── executor             # 低权限本机执行服务，无平台数据库与 GitHub 凭证
└── postgres
```

executor 是同一主机的安全边界，不是多 Worker 调度或新增消息队列。控制请求使用本机受限
通道和按 Run 限定的能力；不提供任意平台命令执行。每个 Run/验证任务使用独立执行组。
app ↔ executor 的控制通道协议（启动 Run、转发工具请求、回收事件、终止执行组、鉴权方式）
在 0c 开始前单独写成附录，本文只定义边界不定义协议。

Workspace 使用显式挂载：

```text
./workspaces:/workspaces
```

PostgreSQL、canonical clone、sealed artifacts/evidence 和必要配置各自使用持久卷；
不能只持久化 workspaces。secret 通过独立受限挂载/提供器注入，不能放入公共 Compose 配置。
app 可直接同源提供构建后的 Web 静态文件，不要求为首版额外引入 Caddy/Nginx。
若已有反向代理可复用，但必须保留源站隔离和 Access JWT 验证。

镜像职责：

```text
app：Rust server、GitHub Adapter、可信策略/证据控制器
executor：codex CLI/app-server（锁定版本）、git、固定 Harness-Gate、node/npm/cargo
LocalGitBroker：受限 Git 操作服务，无 GitHub 凭证，不执行仓库任意 hooks
```

MVP 对外部 Repository 只保证与本项目同类技术栈的基础工具链；其他技术栈必须在其
经批准的 `WORKFLOW.md` after_create hook 中显式准备依赖，并使用独立的依赖网络白名单；
不能假设镜像预装全部生态，也不能让安装脚本在持有平台凭证的 app 中执行。

Codex 认证采用以下任一受控方式：

```text
受控认证代理，或只对 app-server 可读、对工具执行视图不可读的模型认证挂载
禁止把整个 Runtime host 的 CODEX_HOME 或平台 secret 目录挂入执行视图
```

认证能力必须通过 17.3 的隔离测试；不能只以“只读挂载”宣称隔离成功。认证不复制到 workspace，不写入 Agent Context、日志、
`WORKFLOW.md` 或 GitHub token 环境。GitHub App 凭证始终只由 Platform / GitHub Adapter
持有，绝不注入 Agent。

### 21.2 进程模型

Phase 0 / Phase 1 的控制面可以是单个应用进程：

```text
Axum API
Global Scheduler
Supervisor
Reconciler
Handoff Worker
ResourceCleaner（0a：独立异步 worker，持久化任务与补漏）
GitHub read-only poller
Notification Dispatcher
SSE broadcaster（Phase 0b P2 可选）
```

Agent 子进程由 Supervisor 经 executor 启动，执行组清理不依赖控制面正常退出。
Lease 看门狗与进程组终止机制必须能在 API 进程崩溃或失去数据库连接后保持 fail-closed。

### 21.3 多 Worker

当需要多个 Worker 主机时再引入：

```text
Worker leases
remote workspace
SSH / agent transport
Redis Streams / NATS
```

在此之前不要为了“未来扩展”引入分布式 MQ。

### 21.4 Cloudflare 域名、Tunnel 与 Access（Phase 0b 必须）

复用用户已有域名和连接器，不要求公网 IP、路由器端口映射或手机安装 VPN 客户端。
首版部署选用“公开 hostname + Access 保护”的 Web 入口，而不是直接暴露无认证的服务：

```text
电脑 / 手机浏览器
    → HTTPS：factory.<用户域名>
    → Cloudflare Access：只允许本人登录
    → Cloudflare Tunnel：既有 cloudflared 主动建立出站连接
    → 受限源站入口：响应式 Web + /api
    → 控制面 → executor / PostgreSQL / GitHub / 通知渠道
```

部署验收顺序：

1. 创建单一 hostname 与 Access self-hosted application，填写允许身份和会话期；
   在发布源站前先配置拒绝默认访问的策略，不先裸露服务再补认证。
2. 绑定既有 Tunnel 的精确 hostname 路由和拒绝其他流量的兜底规则。
   cloudflared 与 app 在同一主机时用回环绑定，或仅它们可达的私有容器网络；
   连接器在另一台 LAN 主机时限制源地址，并对该段使用可验证的 HTTPS，
   不以“局域网可信”为由开放所有设备直连。
3. app 配置固定 Access issuer、AUD 和用户身份，按 17.4 验证 JWT；尝试绕过 Tunnel
   直接访问、伪造邮箱/身份头、伪造签名、`alg=none` / HS256 混淆、错误 AUD、过期 JWT，
   均必须失败（`spikes/s3` 的用例可直接复用）。复用既有 Tunnel 时只需追加一条 Public Hostname
   到独立端口，注意与同机其他服务的端口冲突。
4. 验证边缘 HTTPS、域名和 Access 登录；不为省事对 app 源站启用 `noTLSVerify`。
   同机回环/受限容器网络的 HTTP 与跨 LAN 的 HTTPS 是明确不同的部署边界。
5. 手机上完成登录、提交、待办审批和通知链接跳转，再验证会话到期后重新登录不会自动批准。
   日常操作共用一个域名与登录入口，不另建二维码配对系统。
6. 在 Access 应用上执行 Revoke existing tokens，手机与电脑刷新均须回到登录页。

Tunnel 只代理控制台入站访问；GitHub API、模型、依赖下载和通知仍需各自批准的出站网络。
禁止把 Executor 的任意预览端口、终端、数据库或 Docker socket 自动挂到这个公开 hostname。
应用运行预览不是首版交付前提；确需手机预览时另定义受认证的预览入口与生命周期。

Phase 0 只做 GitHub 出站轮询，因此没有 webhook 的 Access 例外。Phase 1 选择接入时，
使用独立 webhook hostname 或精确路径，仅路由到 webhook 接收器；GitHub 不需要取得
用户 Access 会话，接收器必须独立验证原始请求体 HMAC、事件类型、仓库范围和 delivery ID。
默认未匹配路由拒绝请求，不能让 webhook bypass 同时开放 `/api`、静态管理页面或健康详情。

官方依据（实施时复核部署界面和协议，不以本文件代替厂商文档）：

```text
Cloudflare Tunnel / public applications:
https://developers.cloudflare.com/tunnel/
Cloudflare Tunnel / outbound-only connections:
https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/
Cloudflare Access / Validate JWTs:
https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/
GitHub REST / best practices:
https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api
```

### 21.5 单机运行、备份与恢复（Phase 0 基础要求）

- 开发机不依赖桌面登录保持服务；配置开机启动和进程重启，停用不适合无人值守的自动休眠，
  但不擅自修改用户主机电源设置。平台离线时不能审批或执行，这一限制必须可见。
- 升级前暂停新调度，处理或显式停止当前 Run，确认执行组静止，再备份数据库、canonical/
  sealed refs、产出清单、权威证据和配置。备份记录 schema/version 与文件清单，
  secret 另行加密备份，不混入仓库或普通日志。
- 首版提供每日备份与升级前备份的可重复脚本/操作说明。需要一致性窗口时暂停写入并确认
  无在途交接后取快照，不承诺对运行中的数据库和文件分别复制就能一致恢复。
  备份失败显示待办；保留至少一份离机、受限且加密的副本，单机目录副本不是灾备。
- 首次发布前至少实际恢复一次到隔离目录/数据库；默认禁用调度和外部写操作，
  避免演练环境产生真实 PR、通知或并发执行。恢复生产时先对账远端和旧执行组再开放写入。
- 从 Phase 0a 起，在启动、调度前及运行中检查磁盘可用空间和执行资源额度；低于可配置阈值暂停新的 dispatch/验证任务并显示
  原因，不当作代码失败消耗模型重试。不自动删除有活动 lease、reservation 或未封存改动的目录。
- 为控制面持久化预留空间；写入返回 ENOSPC 等错误时不得继续 push/PR/merge，恢复后先对账，
  不把写入失败当作成功。最低限度保护不等待 0c 的完整备份；0a 同时实现 10.6 的最小异步清理，
  空间仍不足时保持背压，不能依赖清理一定立即成功。
- 首版支持展示磁盘占用和人工确认后的安全清理；0a 的受管资源按已授权保留策略异步清理，
  无需 Agent 等待或逐次审批。多仓资源协调、共享缓存淘汰和历史数据 GC 留到 Phase 2。
  保留失败现场和恢复所需证据，清理不能破坏已交付 PR 的追溯链。
- 手机断网/Access 会话结束与主机断电分开处理：前者不取消运行；后者恢复后按 5.1 对账。
  Tunnel/通知暂时断开不等于模型或数据库失效，不因此自动重启编码。

---

## 22. 测试和验证矩阵

本章的"Phase 0"标签统一指 0a～0c 合计，具体归属以第 23 章为准：完整租约/OS 隔离/digest 类
属于 0c；最小停止/恢复/工作快照/人工等待与异步清理从 0a 起，Cloudflare/通知/手机类属于 0b，其余属于 0a。三端审批与 CLI 相关条目仅在实现该可选
能力时执行。

零介入是首要验收指标，各 Phase 的集成测试都要统计"介入次数 / 需求数"和介入原因分布；
介入原因不属于 0.1 列举类型的，视为缺陷而不是正常等待。

### 22.1 Domain

- 状态转换合法性
- 非法状态转换拒绝
- 依赖环检测
- 父子 Requirement
- Recovery / ReviewFix 关联
- knowledge_policy 启用和禁用仓库输入两种模式
- Requirement Revision 不可变
- Acceptance Criterion 稳定 ID 和证据绑定
- 未授权外部输入变化产生 InputSnapshotChanged，本次产出合入记录 OutputIntegrated
- 受影响 Requirement 进入 `NeedsRevalidation`
- Contract 未变时自动重验证通过 → 自动恢复原阶段，不产生待办；Contract 变了 → 自动新 Revision 回 Ready
- 自动重验证失败才进待办箱
- 失效且未经重验证授权的 snapshot 不能用于 Done
- Ready 校验拒绝没有 `verification_ref` 的 AC，拒绝未经 Policy 允许的 `manual_review`
- Phase 0 拒绝执行性依赖；Phase 1 校验上游 Done 与基线祖先关系

### 22.2 Scheduler

- Phase 0 / Phase 1：单 Project / 单 Repository / 单 Agent，待审批仍占用容量
- Phase 2：多 Project 并行
- Phase 2：多 Repository 并行
- Phase 0：全局容量固定 1 且不可绕过；Phase 2 再验证配置上限和并行吞吐
- Phase 0：同 Repository 串行
- Phase 0：FIFO + priority
- Project 容量限制（Phase 3）
- Repository 容量限制（Phase 3）
- 公平调度（Phase 3）
- Phase 0：lease 过期不等于可接管，验证旧执行组静止与 dispatch_epoch
- Phase 0：续租失败/数据库中断，完整执行组终止及旧消息隔离
- Phase 0：Running + 未完成交接不被判作孤立状态
- Phase 0：retry backoff
- Phase 0：retry 上限
- Phase 1：同一 PR branch 的 Recovery / ReviewFix 互斥
- Phase 0：单纯暂停领取不取消当前 Run；低磁盘不启动新模型，写入已失败时停止旧组并保全工作
- Phase 0a：blocker 优先于 active/重试余额/进程失联；解除条件与恢复快照未通过不新建 Run

### 22.3 Workspace / Git

- workspace 路径确定性
- project/repository 隔离
- symlink escape
- path traversal
- worktree 创建和清理
- branch 冲突
- dirty workspace
- 进程组终止

### 22.4 Agent Runtime

- initialize handshake
- thread/start
- turn/start
- stream completion
- malformed message
- response timeout
- turn timeout
- stall timeout
- approval policy
- unsupported tool
- token accounting
- stderr 与 stdout 分离
- Phase 0：`report_completion(summary)` 缺失/重复声明处理
- Phase 0：声明先落库再回复；声明后不再接受写入请求
- Phase 0：dynamicTools 注册和准确 request ID 回包
- Phase 0：TokenUsageUpdated 独立累计，重复事件不重复计费
- Phase 0：沙箱内命令/文件修改不产生任何 runtime_request；沙箱外请求默认自动拒绝并记录，Agent 收到拒绝原因后 Run 继续
- Phase 0（16.9）：需求声明 `presets=["cargo"]` 的任务访问 crates.io 成功且不产生待办；未声明域名被代理拒绝且 Run 继续
- Phase 0（16.9）：`--noproxy '*'` 直连 IP、SOCKS 绕过均被沙箱网络隔离挡住
- Phase 0（16.9）：Agent 提出 `network_scope_request` → 需求详情出现 Contract 修订建议；采纳后走 NeedsRevalidation；未采纳不改变 allowlist
- Phase 0（16.9）：平台运行中不因 Agent 请求而放宽 allowlist；新 Run 才读取新集合
- Phase 0：Policy 将某类别设为 ask 时才产生待办；user input 请求始终产生待办
- Phase 0：持久化人工等待超过普通 stall 时限，仍续租且不推进未授权操作
- Phase 0：独立请求超时、Run 绝对寿命上限、Runtime 失联和审批送达未知各自正确处理
- Phase 0：待审批重启后旧请求失效并转人工，不复用历史批准
- Phase 0b：电脑 Web、手机 Web 读取同一个请求，任选一端批准/拒绝/回答后正确回传；并发冲突第一个 CAS 胜出
- 可选 CLI：三端并发冲突、旧操作摘要/版本、重复相同决定、CLI 退出和回复丢失
- 原生 Codex TUI 接入仅做后续兼容性实验；不得以存在 `--remote` 参数代替上述测试

### 22.5 GitHub

- Phase 0：PR 关联、只读 CI/Review 摘要、GitHub 跳转、合并/关闭回显
- Phase 0：轮询条件请求、分页、限流退避、stale/unknown 和最后同步时间
- Phase 0：head 改变后不复用旧 CI 成功，关闭/合并不伪造 Done
- Phase 1：必需 Check 名称集合匹配；缺失 / skipped / neutral 不算通过
- Phase 0（12.3）：伪造引文的失败收据被拒绝（quote 不在绑定脱敏视图、source_sha256 不匹配、status 与
  observed_is_error 不一致、无 fatal/failure 证据各有独立用例）
- Phase 0（12.3）：收据验证失败 → fail-open 回退安全脱敏片段 + summary_unavailable，Run 不失败
- Phase 0（8.4）：上下文降级只降低压缩率；`source_refs` 记录 skipped/裁剪，可追溯到原始内容
- Phase 1：自动 Merge 在 17.5 条件全部成立时触发且仅触发一次；任一条件不成立不触发
- Phase 1：merge_commit_sha 非空但 merged=false 不算已合并，未知结果不伪造成功
- Phase 1：Checks 按受信来源/事件与有效 attempt 分组，旧失败不永久阻塞已成功重跑
- Phase 1：自动 Merge 期间 head 变化 → 放弃本次 MergeOperation，重新等待条件
- Phase 1：合并 API 未知响应先查合并事实，不重复合并
- Phase 1：post_merge 验证失败 → 自动创建 revert 建议进待办箱，不回滚、不自动 revert
- Phase 1：Policy 关闭自动 Merge 时人工 Merge 的 head 绑定、规则检查、重复点击
- Phase 1 可选 Webhook：signature、delivery 去重、乱序和精确回调路由隔离
- Phase 2：多 installation / repository 路由

### 22.6 Recovery

- Phase 0：声明后验证 test/AC 失败 → 执行预算内自动重跑一次带失败摘要的新 Run；仍失败才 Failed
- Phase 1：必需 Check 失败自动创建 CiRecovery，不需要人确认；复用原 PR branch
- Phase 1：未 resolved review thread 自动聚合为一个 ReviewFix；stale 意见跳过并列出
- Phase 1：ReviewFix 等 CI 通过后才运行
- Phase 1：修复预算默认 3，Policy 可配置；耗尽进入 Blocked 并出现在待办箱
- Phase 1：声明后修复无需创建子任务也必须预留 RepairAttempt；formatter 与 Agent 共享一次尝试
- Phase 0a：自动 blocker 探测不唤醒模型；环境恢复自动继续，授权扩大只生成一次待办
- Phase 1：重复事件不重复创建任务
- Phase 1：安全/配置类失败不得自动修复，本项目与外部仓库同一规则
- Phase 1：`require_fix_confirmation` 开启时每次修复前进待办箱；默认关闭不产生待办
- Phase 1：人工 Unblock 按受阻阶段恢复；Unblock 不重置计数
- Phase 2：按失败类型分配预算，升级不放宽历史冻结预算

### 22.7 输入一致性

- Requirement Contract 修改影响分析
- MVP：knowledge_policy 实际读取文件 `path + blob_oid` 变化检测
- Phase 2：Repository policy / workflow / Harness-Gate 完整 digest 变化检测
- 冻结 digest、来源分类和重验证授权（自动 / 人工）联合校验
- Phase 1：修改 WORKFLOW/知识文件的本次 PR 合入不造成自触发死循环
- Phase 1：外部同路径变更不能借本次 PR 获得豁免
- Phase 1：最终合并 commit 与验证证据一致
- Phase 1：`Done` 记录完整 revision、snapshot 和 AC 证据
- Phase 1：同 Revision 多 Run/多 SHA 的证据追加保存，不覆盖历史

### 22.8 本项目门禁与 UI

- 本项目 Harness-Gate `config check`
- 本项目 Harness-Gate `doctor --strict`
- 本项目 Harness-Gate `hook`
- 本项目 Harness-Gate `verify --all`
- Agent 修改 Gate 配置、验证入口或报告不能自证通过
- hooks/安装脚本/测试进程不可读取平台凭证或写入权威 evidence store
- Angular Desktop Chrome 视觉验收
- Angular Pixel 7 视觉验收
- Angular 暗色、键盘、reduced-motion 和异步状态验收
- Phase 0：用户实际手机浏览器的登录、输入法、待办、锁屏返回和通知跳转验收

### 22.9 真实集成测试

Phase 0a 真实集成（localhost）至少验证：

```text
浏览器创建 Requirement（≥1 条 automated_test AC）→ Ready
→ Worktree → Agent 修改 / create_local_commit（沙箱内无任何请求）
→ report_completion 持久化 / 执行组静止 / 验证（含 AC）/ 封存
→ HandoffOperation 幂等 push / 创建 PR → Submitted
→ 全程待办箱为空

再跑一条故意写歧义的需求：Agent 提问 → 待办箱出现 → 回答 → 继续 → PR
```

Phase 0b 追加：手机真实域名 / Access 登录完成上述路径；Agent 提问时手机收到通知并回答；
用户在 GitHub 合并后平台显示 PR=MERGED、最终 SHA、同步时间，内部保持 Submitted。

在声明前、声明后、封存后、push 后和 PR 创建后分别注入重启；验证各自恢复路径，
而不是要求所有重启都创建新 AgentRun。

Phase 0a 同时执行 12.7.5 的预检、验证幂等和磁盘故障场景，采集 19.3 的分项指标；
Phase 1 追加证据复用失效、确定性格式修复与重复失败停止场景，不因节约模型调用减少 required 检查。

Phase 1 真实集成至少需要一个 disposable GitHub Repository（含真实 CI workflow），验证：

```text
创建 10 条评审通过的 Requirement（其中 ≥2 条故意让首次实现的测试失败）
→ 全部自动：Worktree → Commit → Push → PR → CI
→ CI 红的自动 CiRecovery → CI 绿
→ 自动 Merge → post_merge 验证批次 → Done
→ 统计：零介入到 Done 的条数 ≥7；介入原因全部属于 0.1 类型
→ 一条 Policy 关闭自动 Merge 的需求走人工 Merge 路径，验证两条路径共用 MergeOperation 校验
```

真实测试失败不能静默当作通过。

阻塞体验验收：另构造缺依赖、磁盘不足、权限拒绝、未知退出四种任务，验收者只使用任务
详情页，在不打开原始日志、不另开 Agent 会话的条件下说明为何停止（或明确尚未确认）、
已保存什么、系统正在做什么、自己下一步是什么、平台怎样验证、从哪里恢复。任一场景只能
回答“blocked，得找人查日志”即失败。未知故障允许尚无根因，但必须有具体证据缺口与诊断
动作；此验收不要求系统解决任意未知故障，也不允许为了通过测试编造原因。

手机 UI 测试使用测试仓库和安全的受控审批动作；不得为测试而批准访问平台凭证或宿主机
任意路径。云入口和通知渠道在用户实际部署环境验收；CI 模拟测试不能替代真实访问结果。

### 22.10 故障窗口验收矩阵

| 阶段 | 注入场景 | 必须观察到的结果 |
|---|---|---|
| Phase 0 | 无 report_completion 就崩溃，含未提交/未跟踪实现 | 不交接；静止后保存快照，核查 blocker，带恢复上下文按预算新建 Run |
| Phase 0 | 声明已落库，工具回复丢失 | 重复声明幂等，恢复验证，不启动第二个写入者 |
| Phase 0 | 声明后 HEAD 或文件继续变化 | 拒绝封存与交接 |
| Phase 0 | 主进程重启，旧子进程仍存活 | 保留屏障，确认整个旧执行组退出后才接管 |
| Phase 0 | lease 失效后旧完成事件迟到 | 仅归档，不能推进状态或触发 push |
| Phase 0 | push 成功、PR API 超时 | 补查并继续同一交接，不重新编码 |
| Phase 0 | PR 已创建但响应丢失，或重复投递 | 只关联同一个 PR |
| Phase 0 | Cancel/Revision 变更发生在 Sending 前后 | 前者禁止调用；后者归档在途结果，但不进入 Submitted |
| Phase 0 | Agent 改 Gate 配置/验证入口/报告 | 受信策略不被更新，不能通过修改规则绕过失败 |
| Phase 0 | hook 的 test 失败与 secret 失败 | 前者可有限反馈修复；后者立即转人工，无提交 |
| Phase 0 | 审批迟到或工具尝试读取平台凭证 | 回答被拒绝，凭证不可读 |
| Phase 0 | 手机锁屏/断网/Access 会话到期 | 任务不取消，重新登录后读当前待办，不自动重放批准 |
| Phase 0 | 审批等待超过普通 stall 时限 | 仍续租；按独立 human_wait_timeout 处理，不当作卡死重启 |
| Phase 0 | 电脑 Web/手机 Web 同时批准或拒绝 | 第一个有效 CAS 胜出，其余端显示冲突/已处理 |
| 可选 CLI | CLI 在确认后响应丢失/SSH 断开 | 读取既有决定，不换幂等键重发；后台任务和待办不被取消 |
| 可选 CLI | Agent 尝试访问审批 Unix socket/伪造人工 UID | OS 隔离与 peer credential 验证拒绝，不允许自批 |
| Phase 0 | Agent 请求白名单外网络 / workspace 外路径 | 自动拒绝并记录，不产生待办，Run 继续 |
| Phase 0 | 声明后验证适用 AC 失败 | 自动重跑一次带失败摘要的新 Run；仍失败才 Failed 进待办箱 |
| Phase 0 | 决策已落库、Runtime 回复丢失 | 显示送达未确认，核对原 request ID，不重发工具动作 |
| Phase 0 | 带待审批请求的 Run 重启 | 旧请求失效，转人工重试，新 Run 重新请求授权 |
| Phase 0 | 通知服务失败/同一事件重复消费 | 待办仍可处理，通知有限重试，不重启编码或重复决策 |
| Phase 0 | PR 轮询超时/限流/head 变化 | 显示 stale/unknown，不把旧检查当新 head 成功 |
| Phase 0a | 缺依赖/能力不匹配/只读构建目录 | 模型调用为零；保留预检证据，补齐后重新预检 |
| Phase 0a | 同批次验证重复请求/准备阶段重启 | 不并行重复执行；按原阶段恢复，累计预算不重置 |
| Phase 0a | 磁盘不足或持久化 ENOSPC | 暂停领取/外部写入并保全原目录；有声明恢复平台步骤，无声明核查快照后续作 |
| Phase 0a | report_blocker 后 Issue active/诊断文件变化/回复丢失/重启 | 不新增 turn/编码 Run；同一报告幂等，解除条件未满足不恢复 |
| Phase 0a | report_blocker 与 report_completion 并发 | 原子互斥只接受一个结束意图，迟到请求不覆盖 |
| Phase 0a | 恢复快照缺失/损坏/未保存、恢复摘要失败 | 前者不启动模型且保留原目录；后者用结构化恢复上下文降级 |
| Phase 0a | 完成事务后崩溃/清理回执丢失/重复消费 | 清理 task 不丢、同实例幂等，无模型调用 |
| Phase 0a | 清理权限失败/后继任务引用/同路径新实例 | 运维待办或 Deferred；不误删、不回写需求失败，不阻塞独立资源任务 |
| Phase 0a | 输出归档失败/分页跨 UTF-8 边界/跨 Run 读取/原目录清理 | 显式降级、分页完整、授权隔离，有效恢复引用不悬空 |
| Phase 1 可选 | 动作合并编辑失败/检查失败/重复请求 | 不误执行后续命令、不丢修改、不重复编辑，最终验收独立 |
| Phase 1 | 证据身份/有效期/策略资格变化 | 拒绝复用；需要验证时重新执行，不沿用旧 PASS |
| Phase 1 | formatter 修复 / 同输入重复失败 | 新 SHA 重验证；确定性同指纹两次失败后停止第三次自动执行 |
| Phase 0a | Agent 完成后 Gate/AC 失败 | Run 执行成功不改写；独立验证失败，平台按阶段选择修复 |
| Phase 0a | 环境恢复/探测重复投递/探测耗尽 | 单探测幂等；满足后自动续作，耗尽只生成一次待办 |
| Phase 0a | 缺依赖/磁盘满/权限拒绝/交接未知 | 阻塞卡直接说明具体步骤、证据、系统动作、用户下一步、解除判据及恢复阶段 |
| Phase 0a | 无 Agent 报告、固定探针失败或根因未知 | 平台生成部分诊断包；明确 missing_evidence/unknown 和具体采集动作，不只显示错误码 |
| Phase 0a | 用户点已处理但解除检查仍失败 | 同一卡片显示尚缺条件；不重复待办、不启动编码、不改写解决事实 |
| Phase 0a | 旧卡片提交/重复点击/诊断包越权 | CAS/幂等与归属校验生效，不执行过期动作、不泄露日志 |
| Phase 0c | 已知故障与未知故障分别触发诊断 | 前者零诊断模型调用；后者仅在能力/证据/预算满足时触发有限只读诊断 |
| Phase 0c | 模型自造根因/证据/action_id、尝试写代码或联网 | 不能 confirmed/解除 blocker；引用和动作拒绝，写入与越权路径实测拒绝 |
| Phase 0c | 诊断崩溃/取消/模型失败/预算耗尽/重复 bundle | 保留累计额度与固定诊断；不重启编码、不重置预算，迟到报告只审计 |
| Phase 0c | 诊断与编码同时领取 | 共用模型容量与原子锁，并发不超上限，诊断不抢占原 worktree 写入权 |
| Phase 0a | revise 时旧组存活/快照失败/交接在途 | 先撤销保全与对账；不发新写入权，不丢旧工作 |
| Phase 0a | 收据含敏感值、脱敏后分页读取 | 引文核对脱敏视图，raw/view 分别验摘要，Agent 不可读敏感原文 |
| Phase 0a | 多资源暂时清理失败/保留期等待 | 有限重试不逐个通知；永久/耗尽/积压/影响调度才聚合运维待办 |
| Phase 0a | expected 匹配但目标不是其后代 | 禁止 push；不因 lease 正确允许历史改写 |
| Phase 1 | 未合并 PR 有 merge_commit_sha | 不记录 merged，不启动 post_merge，查询未知有限对账 |
| Phase 1 | 同 SHA 的旧失败 attempt 与新成功 attempt | 按可信分组取有效 attempt；保留历史，不永久卡住或误取其他组成功 |
| Phase 1 | 本地/CI/Review 修复交替、formatter 后转 Agent | 共用根 RepairAttempt；不漏计、不双扣、不借子任务绕过上限 |
| Phase 1 | 已处理 thread 未 resolve / 新 head / 回复丢失 | 不重复修复；仅新意见版本或明确复审失败才申请新尝试 |
| Phase 1 | AC 仅适用 post_merge / should AC 在适用阶段失败 | 前者按已批准阶段执行，后者不能因优先级低而跳过 |
| Phase 0 | 伪造 Access 头/错误 AUD/直接访问源站 | 拒绝管理请求，无状态修改 |
| Phase 0 | 磁盘不足/备份失败 | 暂停新领取或告警，保留现场，不消耗模型重试 |
| Phase 1 | PR 修改所选知识文件后合并 | 正确分类自有产出并继续最终验证 |
| Phase 1 | 同路径存在外部输入变化，Contract 未变 | 自动重验证；通过则恢复，失败才进待办箱 |
| Phase 1 | PR CI 成功但最终 SHA 无证据 | 保持 pending，不能 Done |
| Phase 1 | 同一 AC 在两个 SHA 上先成功后失败 | 保留两份历史，最终视图使用正确目标 SHA |
| Phase 1 | 上游仅 Submitted 或不在下游基线 | 不领取下游，不消耗 retry |
| Phase 1 | 自动 Merge 条件成立瞬间 head 变化 | 放弃本次 MergeOperation，按新 head 重新等待 |
| Phase 1 | merge API 响应丢失 | 先查合并事实，不重复合并 |
| Phase 1 | 必需 Check 被 skipped / 缺失 | 不视为通过，不合并，等待或超时 Blocked |
| Phase 1 | post_merge 验证失败 | 不回滚，创建 revert 建议进待办箱 |
| Phase 1 | CI 失败 3 次仍未修好 | 预算耗尽 → Blocked → 待办箱；不再消耗模型 |
| Phase 1 | reviewer 在已被后续提交改掉的行上留意见 | 标记 stale 跳过，摘要中列出，不套用到新版本 |

### 22.11 首版部署验收

- Cloudflare hostname、Access 本人登录、JWT 验证和源站不可绕过；
- 会话撤销后旧客户端请求被拒绝，通知不带认证/批准凭证，敏感 API 不缓存；
- （可选 CLI）平台 CLI 经人工账号/受限 socket 完成同一审批，其他 UID 与 executor 不能访问；
- 首版没有公开 webhook 入口；后续启用时精确 callback 路由不暴露管理 API；
- 数据库、sealed refs、产物清单和证据恢复到隔离环境，外部写操作默认禁用；
- 恢复生产前先对账远端与旧执行组，不重复创建 PR 或并发修改原分支。

---

## 23. 实施分期

本章是实际开发队列，不按前文章节顺序逐章实现。每个 Phase 是一个可独立发布、发布后即可
日常使用的垂直切片；下一个 Phase 不反向扩大上一个 Phase 的范围。

每个 Phase 内的条目分三级：

```text
P0  没有它该 Phase 不能发布
P1  该 Phase 内完成，但可以在 P0 跑通后补
P2  可以顺延到下一 Phase，不影响本 Phase 验收
```

### S. 实施前 spike（每项 1～2 天，先于 Phase 0a）

以下不确定性直接决定第 8、11、12、17 章是否成立，必须先用可运行代码验证，结论写回本文：

| # | 验证内容 | 决定的设计 |
|---|---|---|
| S1 | 锁定版本的 Codex app-server：`dynamicTools` 注册 / `item/tool/call` 回包 / `experimentalApi` 开关；`workspace-write` 沙箱是否真正拒绝写 `.git` | **已完成（2026-09-07），结论见下方 S1 结论与 `spikes/s1/README.md`。** 8.1 的 `create_local_commit` / `report_completion` 成立，退路方案不需要启用 |
| S2 | GitHub App 安装 → installation token → 条件 push → 创建 PR → 读取 Checks → sha 守卫合并 | **已完成（2026-09-08），结论见下方 S2 结论与 `spikes/s2/README.md`。** 11.1 权限集、14.4 步骤 2/3 的实现方式、17.5 合并守卫的错误码由此确定 |
| S3 | Axum 中验证 Cloudflare Access JWT（签名 / issuer / aud / 过期），并实测撤销会话后请求被拒 | **已完成（2026-09-09），结论见下方 S3 结论与 `spikes/s3/README.md`。** 17.4 校验规则、`sub` 绑定、撤销语义由此确定 |
| S4 | Harness-Gate 在本项目上的 `hook` / `verify --profile ci --all` 实际耗时、误报率、JSON 输出稳定性 | **已完成（2026-09-10），结论见下方 S4 结论与 `spikes/s4/README.md`。** 12.6 按 0c 切换；版本锁定改为 0.3.7 |
| S5 | Codex 受限网络模式实测：白名单域名放行、非白名单域名拒绝、绕过路径、审批策略 | **已完成（2026-09-11），见下方 S5 结论与 `spikes/s5/README.md`。** 配置位置定为 `/etc/codex/requirements.toml` 的 `[experimental_network]`（系统级） |
| S6 | 外部方案评审：NVlabs/SoL-Pi 的四项 harness 效率机制 | **2026-09-13 已复评，不引入依赖。** 采纳受控输出归档/分页读取与收据，动作合并单独实验，主动压缩后置；见 8.5、8.6、12.7.6 |

#### S1 结论（2026-09-07，codex-cli 0.153.4，Linux landlock）

1. `dynamicTools` 闭环成立：`initialize.capabilities.experimentalApi=true` → `thread/start.dynamicTools`
   → 服务端 `item/tool/call`（`threadId / turnId / callId / tool / arguments`）→ 客户端回
   `{success, contentItems}`。真实模型运行按顺序收到 `create_local_commit` → `report_completion`。
2. **workspace-write 的可写根 = app-server 进程的 cwd + `/tmp`**；`thread/start` / `command/exec` 的
   `cwd` 参数不增加可写根；只有可写根顶层的 `.git` 受保护。因此**每个 Run 必须单独启动一个
   app-server，以该 Run 的 worktree 为进程 cwd**；从上层目录启动会让整棵树可写、嵌套 `.git` 失去保护。
3. worktree 场景理想：真实 git dir 在 canonical clone 内、不在可写根 → `git add/commit` 以
   `index.lock: Read-only file system` 失败；`.git` 指针文件只读；`git status/log/diff` 正常。
   8.3 验收第 7 条通过。
4. **沙箱不限制读**：Agent shell 可读 `~/.codex/auth.json`、`~/.ssh/`。0a 单进程形态下 Agent 能读到
   平台运行用户的一切；0a 只接管自己可信的仓库，0c 的独立 UID executor 是接管任何非自写代码的前提。
5. 网络：必须每次启动显式 `-c sandbox_workspace_write.network_access=false`（或 Policy 白名单），
   用户 `config.toml` 的全局值不可信。
6. `approvalPolicy="never"` 下无任何 `*/requestApproval` 请求；沙箱拒绝作为命令错误回给模型，
   模型据此报告并继续——"沙箱外默认拒绝"路径在协议层可行，S5 只需测失败率。
7. 工具描述要写中性事实。描述里写"这是唯一提交方式"时，模型把它当禁令，拒绝执行提示词明确要求的
   直接 commit 测试。禁令放 developer instructions 或由沙箱强制。

脚本与完整 JSONL 日志见 `spikes/s1/`。未覆盖：macOS seatbelt、多 turn 长会话下的并行工具调用、
`turn/interrupt` 后挂起 `item/tool/call` 的终结（8.3 验收第 4 条）。

#### S2 结论（2026-09-08，App `my-disposable-bot`，仓库 `musutrade/disposable`，public）

1. 最小权限 `contents: write` + `pull_requests: write` + `metadata: read` 跑通全流程：token 换取、
   条件 push、找/建 PR、读 check-runs / check-suites / combined status / actions runs、sha 守卫合并。
   `branches/:b/protection` 403，但 rulesets API 与 `mergeable_state` 可用，不需要 administration。
2. installation token 1 小时；可按仓库限定、按权限收窄；申请未持有权限 422。
3. `--force-with-lease=<ref>:<expected>` 是可靠的条件更新：不符时 `! [rejected] ... (stale info)` rc=1。
   **远端已是目标 SHA 时 git 不检查 lease**（expected 写错也 rc=0），lease 只是更新守卫，
   不是远端状态断言；幂等成功后仍要 fetch 确认。REST 替代路径 `PATCH /git/refs` `force:false`
   非 fast-forward → 422。
4. PR 查找 `state=all&head=<owner>:<branch>&base=<default>` 精确命中；重复创建 422
   "A pull request already exists for <owner>:<branch>."；合并后仍能按同一查询找到（state=closed）。
5. 无 workflow 的仓库 combined status 永远 `pending`、check-runs 为 0——证实 17.5"缺失不算通过"必要。
6. `PUT /pulls/:n/merge` 带 `sha`：不符 → **409** "Head branch was modified"；相符 → 200；
   **已合并后重复调用 → 200 幂等**返回同一 merge sha。合并事实用 `GET /pulls/:n/merge` 204/404 判定。
7. ETag / `If-None-Match` → 304 且不扣限额；限额 15000/h；`GET /rate_limit` 对 installation token 404。

未验证：private 仓库是否需显式 `checks: read` / `actions: read`；真实 workflow 下 check-run 的 `name`
形态；有 ruleset 时 `mergeable_state` 从 `blocked` 到 `clean` 的序列；token 过期瞬间的错误码。

#### S3 结论（2026-09-09，axum 0.8 / jsonwebtoken 11，既有 Tunnel + Access One-time PIN）

1. Rust 侧验签可用 `jsonwebtoken 11`（`DecodingKey::from_jwk` + `Validation`），**必须开 feature
   `rust_crypto` 或 `aws_lc_rs`**，否则编译通过、首次验签 panic。`reqwest` 用 `rustls-tls`。
2. 真实 JWT claims：`aud[]`、`iss`、`sub`（稳定身份 ID，跨设备相同）、`email`、`exp/iat/nbf`、
   `identity_nonce`（每次登录不同）、`policy_id`、`type:"app"`、`country`。**平台 `owner_id` 绑 `sub`。**
3. 本机负面用例全部 401：无 header、仅邮箱头、垃圾 JWT、伪造签名+真实 kid、`alg=none`、HS256 混淆、
   过期、错 aud、错 iss。源站绑 loopback，LAN 地址不可达。未登录访问边缘直接 302，不到源站。
4. 电脑与手机 OTP 登录后均 200；Revoke existing tokens 后两端刷新均回到登录页。
5. **撤销只在边缘生效**，源站对既有 JWT 的判定在 `exp` 前不变 → "源站仅经 Tunnel 可达"是前提。
   本次应用实际会话 12 小时，上线按 17.4 改 8 小时。
6. 复用既有 Tunnel 只需追加 Public Hostname；原计划端口 8787 与同机服务冲突，改用 8790。

未验证：已撤销 JWT 直接重放源站（逻辑上必然通过，未导出用户 JWT 实测）；自然过期边缘行为；
Access service token；非 OTP IdP 下 `sub` 稳定性；JWKS 轮换期。

#### S5 结论（2026-09-10 初测，2026-09-11 补测完成；codex-cli 0.153.4，受限网络模式）

1. `features.network_proxy = true` 后，沙箱任务获得本地 CONNECT 代理：`http_proxy` / `https_proxy`
   指向 `127.0.0.1` 随机端口（另有 SOCKS5 端口）。**不允许的域名在代理层被拒**：
   `curl: (7) CONNECT tunnel failed, response 403` + `Network access to "<domain>" was blocked:
   domain is not on the allowlist for the current sandbox mode.`
2. 该拒绝**不产生审批请求**：`approvalPolicy = "never"` 下三个域名全部被拒、turn 正常结束，
   Agent 拿到明确错误文本。16.9 要求的"沙箱外默认拒绝、不打断人"在协议层成立。
3. 直连绕过无效：`curl --noproxy '*'` 打裸 IP 超时（沙箱网络隔离），SOCKS5 UDP / 非 HTTPS TCP 被禁。
4. 平台自有 CODEX_HOME（不复用 `~/.codex`）是可行且必要的：用户 `config.toml` 里的
   `network_access` / 代理环境会被覆盖或需显式传递；平台须把上游代理通过 app-server 进程环境传入。
5. **allowlist 的配置位置已定位（2026-09-11 补测完成）**：写在系统级
   **`/etc/codex/requirements.toml`** 的 **`[experimental_network]`** 表下（不是 `[network]`，
   不是 `$CODEX_HOME/requirements.toml`，不是用户 `config.toml`），需 root。
   `configRequirements/read` 可回读确认为 `network.domains`。
6. 生效验证：managed 白名单加载后，**`crates.io` 与 `index.crates.io` 真实放行（200）**，
   未列入的 `example.com` / `api.github.com` 报
   `blocked by policy`。即 S5 的机制、配置位置、拒绝语义三者全部落地。
7. 该配置是**全局**的（影响同机所有 codex 进程），不是 per-Run；平台若按 16.9 做 per-Run 生效集合，
   需要自己管理该文件并在 Run 前后收敛，或者等待 Codex 提供 per-session 白名单。
   这是 16.9 实现方案里必须写清的一条约束。

未验证（S5）：`managed_allowed_domains_only` 与用户级 allowlist 的叠加行为；deny 覆盖 allow 的实测；
子域是否需逐条列出（`sub.crates.io` 无真实解析，未能干净验证）；白名单是否区分端口。
另注：上游代理本身可能对目标返回 403（本项目环境 `static.crates.io` 即如此），
排查"是否被沙箱拒绝"要看响应体是否含 `blocked by policy`，不能只看状态码。

#### S4 结论（2026-09-10，harness-gate 0.3.7 源码构建，generic preset）

1. 四项命令在本项目跑通且很快（0.05s / 0.20s / 0.16～0.51s / 0.36～0.38s；仓库几乎无代码，属下限基线）。
2. **`hook` 读暂存区（index）而非工作区**：配置物化到 `$TMPDIR/harness-gate-staged-<pid>-<ts>/` 再校验。
   这是 12.6.1 要的受信快照语义；但配置未暂存时报 `E1000: read staged workflow configuration ... No such file`，
   平台必须在每次 `hook` 前确保配置已暂存并检查退出码，否则门禁静默失效。
3. 拦截能力已实测：真实形态 AWS key → `SECRET_SCAN_FAILURE` + `TEST_SUMMARY: FAIL`；暂存行尾空白 →
   `staged Git whitespace check FAIL`。占位符（`...EXAMPLE`）被正确豁免，不是漏报。
4. 每次 invocation 记 `executor_version`、`input_mode`、`source_identity`、`configuration_digest`、
   `execution_root`，每个 step 带原生 `failure_code`。**`configuration_digest` + `source_identity` 就是
   方案要的"验证策略身份 / 被测源身份"分离**，字段名以此为准。
5. 本机 `cargo install` 的 harness-gate 是 **0.1.0**，与源码仓库 0.3.7 不同；平台必须记录实际执行
   二进制的摘要，不能只记版本号。
6. **自证通过已实测成立（2026-09-11 补测）**：保留 step id、把步骤命令改成恒真（`program="true"`），
   `config check` 通过、`hook` 报 `TEST_SUMMARY: PASS`，尽管暂存区存在违规。删掉 `required_steps`
   引用无效（该列表是"不可豁免"，不是"要跑什么"）；删掉整个 `[[steps]]` 块会 fail-closed。
   结论：**harness-gate 不防自证通过**，12.6.1 要求的"平台用独立批准的 policy revision 比对
   `configuration_digest`、不一致即拒绝采信"是必须实现的硬性控制，不能只靠工具自报 PASS。

#### S2 补测结论（2026-09-10，disposable 仓库含真实 Actions workflow 与 ruleset）

1. check-run `name` == workflow 里 job 的 `name`（`test-job`），发布 App `github-actions`；
   17.5 的必需 Check 集合直接配 job 名，无需 `CI /` 前缀。
2. **combined status 永远是 `pending`**（Actions 只写 check-runs）：绝不能作为 CI 判据。
3. `mergeable_state` 序列实测 `null`（t≈12s，GitHub 仍在计算）→ `blocked`（t≈21s）→ `clean`（t≈30s）；
   Reconciler 必须处理 `mergeable == null`，且 `blocked` 不等于失败。
4. 同一 SHA 返回 **2 条同名 `test-job`**（push 与 pull_request 各一次）；两类触发不能只取第一条。
   判据按 17.5 的受信 selector/事件分组与有效 attempt 选择；补测没有证明必须要求所有历史
   attempt 成功。无法识别替代关系时保持 unknown，不能用任意同名成功掩盖当前失败。
5. 合并事实：仅接受明确 `merged=true` 或 `GET /pulls/:n/merge` 的 204；null/缺失/查询失败
   为未知，有限退避对账。`merge_commit_sha` 非空不能证明合并，未合并 PR 也有测试合并 SHA。
   确认合并后再读取最终 SHA，不以测试合并 SHA 作为 post_merge 目标；sha 守卫不符仍为 409。
   依据：https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request（2026-09-13 复核并修正旧推断）。

#### S6 记录：NVlabs/SoL-Pi 复评（2026-09-13）

本次只读核对当前 README、源码和技术报告，未安装扩展或在本项目运行其基准。SoL-Pi 面向 Pi
的公开扩展接口，不能直接作为 CodexSymphony/Codex Runtime 插件使用；借鉴机制，不新增依赖。

| 机制 | 本项目决定与分期 |
|---|---|
| ObservationPack | 新增采纳：0a 先实现平台受控输出归档与分页读取（8.5），原始结果保留；内部历史投影需另验 Runtime 能力 |
| Evidence-Preserving Reducer | 保留 12.3：规则优先、可验证引文、失败降级；已验证收据不再被二次压缩 |
| Action Fusion | 修正旧拒绝结论：编辑后的快速反馈不等于最终验收；Phase 1 P1 做受控合并实验，保留独立门禁（12.7.6） |
| Online Context Compact | 修正旧“纯收益”判断：需计摘要/缓存重建成本；不依赖 Planner，但仍为 Phase 2 可选实验（8.6） |
| 自动研究方法 | 不照搬大规模搜索；采用单项开关、冻结基线、独立验收集、配对对照和组合回归（12.7.6） |

2026-09-11 的评审已采纳证据保全收据，但漏评 ObservationPack，把 Action Fusion 判为必然
违反受信门禁边界、把主动压缩判为纯收益，均由本次复评修正。工具序列合并可以只影响开发
反馈；是否允许最终交付仍取决于平台独立证据。上下文压缩可能破坏缓存收益，需要测量。

SoL-Pi 当前 ObservationPack 在 Pi 的 context 投影层替换输出并提供 obs_recall，保留会话历史，
归档/压缩失败保留原输出；本项目 8.5 只承诺受控接口可实现的范围。其归档不会在会话结束时
自动删除，本项目接入 10.6 的引用与异步回收，不能复制无期限留存的运维行为。
技术报告披露组合机制约保留 Pi 平均分的 94%；它采用能力容忍范围，不能替代本项目既有
required AC 和安全验收。所有收益需在本项目上实测，不直接采用报告中的节省比例。

核对来源（2026-09-13 的 main/公开报告，未来实现时固定并复核对应版本）：

- [README 与存储行为](https://github.com/NVlabs/SoL-Pi)
- [ObservationPack 实现](https://github.com/NVlabs/SoL-Pi/blob/main/src/sol-pi/extensions/observation-pack/index.ts)
- [失败收据排除与观察身份](https://github.com/NVlabs/SoL-Pi/blob/main/src/sol-pi/extensions/observation-pack/observation.ts)
- [Action Fusion 实现](https://github.com/NVlabs/SoL-Pi/blob/main/src/sol-pi/extensions/action-fusion/index.ts)
- [压缩经济性判断](https://github.com/NVlabs/SoL-Pi/blob/main/src/sol-pi/extensions/online-context-compact/economics.ts)
- [Pi 接口兼容性与压缩后续执行](https://github.com/NVlabs/SoL-Pi/blob/main/docs/compatibility.md)
- [技术报告、实验方法与组合结果](https://nvlabs.github.io/SoL-Pi/)

### Phase 0a：本机闭环（目标 L1）

目标：在开发机 localhost 上，从写好的 Requirement 到平台创建 PR，全程无人介入。
第一条真实需求跑通即发布并开始日常使用。

**P0**

- PostgreSQL + 单个 Rust 进程：API、Scheduler、Supervisor、Handoff Worker 同进程
- Project / Repository / Requirement / RequirementRevision / AgentRun 基础表；多 Project 数据模型，
  使用限制为单 Project 单 Repository
- Requirement Contract：结构化表单 → JSON；AC ID 自动生成；`verification_method` 默认
  `automated_test`，每条 AC 必须有可解析的 `verification_ref`；`manual_review` 需显式选择并给理由
- 评审通过 = `Ready`：`POST /ready` 校验 Contract schema 与 AC 可验证性后直接进入调度队列，
  没有第二次"确认执行"
- 状态机：`Draft → Ready → Running → Submitted / Failed / Cancelled`；Running
  / Submitted 时拒绝普通 PATCH 改 Contract；4.1 的 revise 命令一次授权后保全/对账再应用，
  不实现 `NeedsRevalidation` 主状态
- 调度：全局并发 1，`priority DESC, created_at ASC`；单一 `requirements.lease_token / leased_until`
  + `agent_runs` 活跃部分唯一索引；实现最小启动恢复闸门，完整 Repository lease/epoch/屏障留 0c
- CodexRuntime：spawn app-server、initialize、thread/start、turn/start、事件流、token 去重、
  turn/interrupt、startup / response / stall timeout；schema codegen，版本锁定
- 审批策略冻结为：沙箱内文件修改与命令自动放行；沙箱外请求自动拒绝并记录 `agent_event`；
  Agent 提出的人工问题（user input）→ Run 暂停、Requirement 保持 `Running`、进入待办箱
- Workspace / Worktree：路径 containment、`git fetch` / `worktree add` / 分支 `ai/req-*`、
  after_create hook、成功后保留 72 小时
- 12.7.1 模型启动前的真实沙箱预检；固定依赖/工具能力、可写目录、Broker、磁盘检查；
  准备失败只重试准备，不消耗编码预算；最小低磁盘暂停和持久化失败阻止外部写入
- `create_local_commit` + `report_completion` + `report_blocker` 动态工具；结束意图原子互斥，阻塞落库后不续轮
- 8.1 持久化 auto_probe/manual 分类、有限探测与自动解除；条件满足从 RecoveryPlan 续作，
  需要新授权/未知原因/探测耗尽才聚合一次待办；人工等待时限 2 小时、Run 绝对寿命 8 小时
- 16.9 网络声明与主机级策略资源占用/最小特权配置；可继续的拒绝无待办，范围扩大只请求一次
- LocalGitBroker 用固定 Git 二进制和参数
  数组、禁用仓库 hooks 与 credential helper；声明先落库再回复工具
- 声明后：中断 session、确认旧组静止和 HEAD/工作树完整，保存 candidate 后 Run Succeeded；
  独立执行 Repository Policy 的 custom 门禁（例如 cargo test）与适用 AC，通过才封存，
  验证失败不改写 Run 状态
- HandoffOperation + outbox：条件 push（仅 fast-forward）→ 按 repo+branch 查找或创建 PR →
  持久化关联 → `Submitted`；重试用相同 action_key，不新建 AgentRun
- 只读 GitHub 轮询（60 秒）：已关联 PR 的状态、head SHA、Checks 摘要、合并 / 关闭事实；
  显示 `last_synced_at` 与 stale
- 最简 Web（Angular，localhost）：需求表单、需求列表 / 详情、Run 事件时间线、待办箱（人工问题、
  需人工的失败/运维项）；不做设计系统验收，用 Material 默认主题
- 错误码表子集：`preparation_*`、`agent_*`、`git_*`、`model_*`、`github_*`；分阶段自动重试白名单，
  每阶段预算 2 次；未知错误 fail-closed 为 `Failed + manual_action_required`
- 结构化日志 + `agent_events` / `requirement_events`
- 12.7.3 同批次验证幂等；12.3/12.7.4 规则优先的修复包、原始证据按需读取、恢复事件与累计预算
- 3.7 阻塞/完整工作快照/恢复计划；8.4 每个恢复 Run 必需 Recovery Context，含未提交/未跟踪实现
- 3.8/8.7 DiagnosticBundle、固定诊断与报告、action catalog 和 18.4 可操作阻塞卡；
  已知故障直接给出下一步，未知故障给出已排除项/缺失证据/具体采集动作；不依赖额外 Agent 会话
- 进程重启按 5.1 最小规则对账；存储或快照故障保留原目录，禁止从干净基线自动重跑
- 19.3 模型调用/token 分项、准备/编码/验证/等待/交接耗时与验证去重计数
- 8.5 受控输出归档 + read_observation 分页、失败收据不二次压缩、恢复/验收引用保护；
  12.7.6 单项开关与同验收强度的基线对照，0a 不要求 Runtime 内部历史投影
- 10.6 ManagedResource + 引用登记、CleanupTask/outbox 同事务、后台 ResourceCleaner；
  单仓限额并发、幂等重试、5 分钟补漏，到期 worktree 与一次性测试资源分别回收

**P1**

- 手动 `Stop` / `Cancel` / `Retry`（先核查 blocker；有声明恢复验证，有 manifest 只交接，
  无声明则预检/快照/Recovery Context 通过后新 Run 续作，不默认从头编码）
- Token 与估算成本记录
- `WORKFLOW.md` 读取（prompt、hooks、测试命令），`workflow_hash` 记录

**P2**

- `knowledge_policy` 透传读取仓库文档（20 文件 / 100KB / 1MB）
- Requirement 列表筛选、事件分页

**不包含**

Cloudflare / 通知 / CLI / executor 容器 / Harness-Gate 必选 / 三套租约 / digest 漂移 /
备份演练 / 设计系统验收 / 任何 Phase 1 以后的状态。

**验收**

```text
在开发机浏览器写一条真实需求（含 ≥1 条 automated_test AC）→ Ready
→ 无人介入 → 平台创建 PR → 浏览器看到 PR 链接和 CI 摘要
→ 重复 5 条不同需求，其中至少 3 条零介入到 PR；
  介入的原因都属于 0.1 列举的异常类型，且都出现在待办箱而非日志里
→ 在声明前 / 声明后 / push 后各注入一次进程重启，恢复路径正确，不重复创建 PR
→ 12.7.5 的 0a 场景通过：准备失败零模型调用、重复验证去重、交接不重启编码、ENOSPC 停止外部写入
→ blocker 受理后零续轮；恢复未提交/未跟踪工作和进度，快照无效不启动模型
→ 10.6.4 清理故障场景通过；旧 Issue 清理延迟不占 Agent slot、不阻塞独立的新任务
```

### Phase 0b：远程可用（L1 + 手机）

目标：不在开发机前也能写需求、处理异常。

**P0**

- 既有 Cloudflare Tunnel + Access；Axum 验证 Access JWT；源站只接受 Tunnel 流量；CSRF、`no-store`
- 手机响应式：需求表单、待办箱、Run 详情、人工问题回复、Stop / Retry
- 一个真实外部通知渠道（部署时选定）：待办箱新增项、执行失败、PR 已创建；通知只带链接不带令牌；
  独立重试，失败不影响 Run
- 会话到期 / 撤销实测；写请求不自动重放；多设备重复提交幂等
- 后台不依赖浏览器：关闭页面、锁屏不影响 Run
- WorkspaceReconciler 对账孤儿 worktree，向 0a 的清理队列提交候选；保留已有 GitHub 轮询

**P1**

- 本项目 Angular 设计 Token、页面骨架、暗色、reduced-motion
- 待办箱刷新 ≤10 秒；PR 事实同步 ≤2 个轮询周期

**P2**

- SSE 替代轮询
- 平台 CLI（`factory inbox / answer`），复用 API，Unix socket + peer UID 认证按 17.6

**验收**

```text
手机通过真实域名登录 → 写需求 → Ready → 收到"PR 已创建"通知
→ 一条需求中 Agent 提问 → 手机收到通知 → 回答 → 继续 → PR
→ 伪造 Access 头 / 错误 AUD / 直连源站均被拒绝
```

### Phase 0c：加固（L1 + 安全与恢复）

目标：把 0a 里为了跑通而简化的安全与恢复语义补齐。这一步之后系统才可以接管不完全可信的仓库。

**P0**

- 低权限 executor：Agent、hooks、Gate / test 子进程以独立 UID / 容器运行；实测不能读取平台数据库
  凭证、GitHub App 私钥、Cloudflare 凭证、其他 workspace；执行组按 cgroup / 进程组整体终止
- Codex 模型认证与工具执行视图隔离（受控代理或不可读挂载），按 17.3 实测
- 8.7 平台内只读诊断 Agent：固定诊断不足才自动触发，既有 Runtime、独立预算与共享容量，
  bundle 输入/受控分页、禁止写入和权限变更；模型失效仍能查看和处理固定诊断卡片
- 受信 Gate policy revision：批准 / 撤销 API；Agent 修改 `.harness-gate/`、hooks、验证入口只作为
  待批准变更；本项目 dogfood 切换 `gate_provider = harness-gate`，`hook` + `verify --profile ci --all`，
  fail-closed 规则按 12.6.4
- 完整租约语义：Repository dispatch lease、dispatch_epoch、worktree_leases（branch reservation）、
  恢复屏障、旧 epoch 消息只归档；ProcessReconciler 冷启动 9 条规则（5.1）
- `context_input_digest`（Contract + AC + WORKFLOW.md + 实际读取文件 blob_oid）与
  `NeedsRevalidation`：承载 0a 已有受控输入替代流程
- 22.10 故障窗口矩阵中 Phase 0 标注的场景全部通过
- 每日备份脚本、升级前备份、一次隔离环境真实恢复（默认禁用外部写）；扩展 0a 已有的磁盘保护

**P1**

- 扩展 0b WorkspaceReconciler 的完整 lease/epoch 对账，按 10.6 提交清理候选；远端 branch 不自动删除
- 加固 0a 已有等待/绝对时限与执行组隔离的联动，不推迟最小等待规则

**验收**

```text
22.10 Phase 0 场景 + 22.11 部署验收全部通过；
本项目自身的一条需求经 Harness-Gate 全流程到 PR
```

### Phase 1：自动闭环（目标 L2）

目标：评审通过后，正常路径上人不再介入，包括合并和 Done。

**P0**

- 状态机补齐：`Submitted → WaitingCI → WaitingReview → Merging → Done`，`Blocked`，人工 `Unblock`
- GitHubReconciler 升级为业务对账：精确 head SHA 的 Checks 结果驱动 `WaitingCI`
- **自动合并**（默认开启，Repository Policy 可关）：条件 = 必需 Checks 全部 success
  AND 受信 Gate 通过 AND 所有 pre_merge 适用 AC 在 PR head 上 `verified` 或 `waived` AND 无未解决 review thread
  AND 无进行中 RepairAttempt/写入/验证封存/交接；平台以 expected head SHA 调用 merge，未知结果先查合并事实
- **自动 Done**：合并后建立 `post_merge` VerificationBatch，在隔离环境检出 merged SHA，
  运行 automated_test / gate_check；ci_check 等待默认分支上该 SHA 的结果；全部满足 → `Done`。
  `manual_review` AC 仅在 Policy 显式开启人工验收时出现，出现即进待办箱
- **自动 CI 修复**：Checks 失败 → 提取 Failure Summary（12.3）→ 创建 `CiRecovery`，复用原分支；
  预算来自 Repository Policy，默认每根需求 3 次，串行；耗尽 → `Blocked` + 待办箱。
  secret / 架构 / 配置类失败永不自动修复
- **ReviewFix**：仅尚未处理的意见版本自动聚合；处理后等待复审/resolve 不重复唤醒，
  新反馈才申请新尝试；与声明后修复、CiRecovery 共用 RepairAttempt 预算
- 同 Repository 执行性依赖（`depends_on`）：上游 `Done` 且 merged SHA 是下游基线祖先才领取
- `OutputIntegrated` 来源分类：本次 PR 修改 WORKFLOW / 知识文件不触发自身 NeedsRevalidation
- 12.7 确定性流程接管：验证身份/证据复用（provider 能力不足时完整执行）、受控 formatter
  的 DeterministicRepair/candidate 来源链（不创建 AgentRun），
  新 SHA 完整重验证、重复失败停止规则；统一 RepairAttempt 先预留再执行，等待/对账不调用编码模型

**P1**

- 可选自动 reviewer：独立只读 Agent 对 PR diff 产生 review comments，供 ReviewFix 消费
- 12.7.6 受控 Action Fusion 实验：先验证现有批量工具的收益空间，固定快速检查；
  同一授权、请求幂等、失败保留编辑，结果不替代最终验收；实测无收益则不开启
- Webhook 接收（HMAC、delivery 去重、精确路由）加速对账；轮询保留为兜底
- 平台内人工 Merge 按钮（Policy 关闭自动合并时使用）

**P2**

- 手机逐项人工验收页（仅 Policy 开启 manual_review 时）

**验收**

```text
连续 10 条评审通过的需求：
→ ≥7 条零介入到 Done（含至少 2 条经历过 CI 失败自动修复）
→ 其余介入原因均在待办箱且属于 0.1 的异常类型
→ head 改变后旧合并条件失效；预算耗尽后不再消耗模型
→ 12.7.5 的 Phase 1 场景通过；复用/确定性修复保持相同 required 检查与 AC 强度
→ 22.10 Phase 1 场景全部通过
```

### Phase 2：降低评审成本 + 多仓（目标 L3）

**P0**

- 描述 → Contract 草稿：用户只写自然语言，系统生成 problem / goal / scope / AC（含
  verification_method 与 verification_ref 建议）；用户评审修改后 Ready
- 澄清 Agent：评审前对草稿挑刺（歧义、不可验证的 AC、与仓库现状冲突），输出问题列表，
  用户回答后更新草稿；不自动 Ready
- 多 Project / Repository 调度：全局并发默认 1，可配置到 4，同仓串行；GitHub App 多安装路由
- 完整输入 digest（Policy、Harness-Gate 配置、完整上下文）与 DigestReconciler
- Recovery 预算与 Failure Summary 策略按仓库配置；Review 意见聚合

**P1**

- 扩展 0a ResourceCleaner：多仓资源协调、共享缓存淘汰和历史数据 GC
- 多仓恢复测试、主机资源配额
- 8.6 主动上下文压缩可选实验：真实长 Run 瓶颈、公开接口验证、经济性判断与独立对照通过后启用

### Phase 3：可选编排

- Discussion 与从对话生成 Contract
- Planner：大需求拆子需求草稿，人评审后进入队列；子任务依赖、集成任务、失败子任务单独重试
- Project 级容量与公平调度（多项目出现饥饿时）
- 事件溯源重放

没有真实需要时可以不实施本阶段。

### Phase 4：扩展

- 多 Runtime（Claude、其他）、Model Policy、fallback、金额预算
- Remote Worker、多机调度、Redis Streams / NATS
- 完整 PWA、Web Push、多通知渠道
- 同一 Requirement 跨 Repository、自动部署

### 推迟清单

| 能力 | 推迟到 | 回归条件 |
|---|---|---|
| Cloudflare Access / 手机 / 通知 | 0b | 0a 第一条真实需求到 PR |
| 低权限 executor、受信 Gate、完整租约、digest 漂移、备份 | 0c | 0b 远程可用 |
| Harness-Gate 作为本项目必选门禁 | 0c | S4 通过 |
| 自动合并、自动 Done、自动 CI 修复、ReviewFix | 1 | 0c 故障矩阵通过 |
| Webhook / SSE | 1 P1 / 0b P2 | 轮询延迟成为实际问题 |
| 平台 CLI 与 socket 认证 | 0b P2 | 需要在终端处理待办 |
| 描述→Contract、澄清 Agent | 2 | L2 达成，评审成为瓶颈 |
| 多仓并行 | 2 | 单仓日常使用稳定 |
| Planner / Discussion / 公平调度 | 3 | 有实际拆解需求或饥饿现象 |
| 多 Runtime / fallback / 预算 / Remote Worker / PWA | 4 | 前序 Phase 稳定 |

### 里程碑顺序

不承诺日历工期，按可演示结果推进；括号内是单人全职的量级估计，用于判断范围是否合理，
不是承诺：

```text
S     spike S1～S5 结论写回文档                                   （1～2 周）
0a    localhost：需求 → 无人介入 → PR                            （3～5 周）
0b    手机：远程写需求、收通知、处理异常                          （1～2 周）
0c    加固：隔离、受信 Gate、完整恢复、备份                       （3～5 周）
1     自动合并、自动 Done、自动修复                               （3～4 周）
2     描述→Contract、澄清 Agent、多仓                             （3～4 周）
```

0a 结束时预期本文至少 1/3 的规则需要根据真实运行修正；修正在 0b 之前完成，
不带着已知偏差进入远程部署。

---

## 24. V1 Definition of Done

V1 = Phase 0a + 0b + 0c + Phase 1。每个 Phase 有独立的发布门槛，前一个 Phase 通过即可开始日常
使用；本章是各 Phase 的最终清单，条目与第 23 章 P0 项一一对应。

贯穿所有 Phase 的首要指标是**介入率**：每个 Phase 的验收集成测试都要记录
"需要人处理的次数 / 需求数"和介入原因；原因不在 0.1 列举类型内的，算缺陷。

### Phase 0a：本机闭环

- [ ] 浏览器（localhost）创建 Requirement：结构化 Contract、自动 AC ID、不可变 Revision
- [ ] `verification_method` 默认 `automated_test`；Ready 校验拒绝无 `verification_ref` 的 AC
- [ ] `POST /ready` = 评审通过，直接进入调度队列，无第二次确认
- [ ] 状态机 `Draft / Ready / Running / Submitted / Failed / Cancelled`；Running/Submitted 拒绝普通 PATCH；revise 一次授权后保全工作与对账再应用
- [ ] 单进程、全局并发 1、FIFO + priority、单一 Requirement lease + 活跃 Run 部分唯一索引
- [ ] CodexRuntime：8.3 验收 1～10 通过；schema codegen、版本锁定
- [ ] 沙箱内操作自动放行、沙箱外自动拒绝并记录；user input 进待办箱
- [ ] Worktree 创建/清理、`ai/req-*` 分支、after_create hook
- [ ] 10.6 受管资源登记/引用、终态事务写 CleanupTask/outbox、独立后台 worker、幂等重试与补漏
- [ ] 清理不调用模型、不占 Agent slot；10.6.4 崩溃/重复/权限/引用/同路径新实例场景通过
- [ ] 12.7.1 真实沙箱预检、独立准备预算；缺依赖/能力不匹配零模型调用，补齐后恢复
- [ ] 最小磁盘保护与 ENOSPC 注入：暂停领取/外部写入，恢复后对账，不消耗编码预算
- [ ] `create_local_commit` + `report_completion` + `report_blocker` 动态工具闭环；结束意图原子落库再回复
- [ ] 运行中 blocker 受理后不续轮，active/诊断文件/重启不绕过；解除条件未满足不调用模型
- [ ] auto_probe 有上限与 deadline，条件满足自动续作；人工问题/安全撤销/权限扩大不自动解除
- [ ] 人工等待时限 2h、Run 绝对寿命 8h，从 0a 支持，重启/超时不重复请求或绕过授权
- [ ] 3.7 工作快照保留 index/未提交/未跟踪实现；每个恢复 Run 带 8.4 Recovery Context
- [ ] 3.8/8.7 固定诊断包与报告、原因确认程度、缺失证据/下一动作；不要求用户另外找日志
- [ ] 18.4 阻塞卡六项问题可直接回答，具体操作/主体/范围/解除判据/恢复位置完整；仅 Blocked 不通过
- [ ] blocker API 版本/CAS/动作白名单、诊断包脱敏与越权拒绝，补充信息不复用旧 Runtime 请求
- [ ] 快照失败保全原目录并禁止自动重跑；摘要失败可降级；无声明新 Run 从已保存进度续作
- [ ] 16.9 网络授权：需求声明生效集合 → 生成 Run allowlist；单次白名单外拒绝可继续时无待办；
      `--noproxy` / SOCKS 绕过被挡；无法继续且需扩大范围时只生成一次修订待办，采用新授权恢复
- [ ] 声明后：中断 session、确认子进程退出、HEAD 校验、保存 candidate 并完成 Run；独立 custom 门禁 + 适用 AC 本地验证，通过才封存
- [ ] 声明后验证失败 → 执行预算内自动重跑一次带失败摘要的新 Run
- [ ] HandoffOperation + outbox：条件 push、查找/创建 PR、持久化关联、`Submitted`；重试不新建 Run
- [ ] 只读 GitHub 轮询：PR 状态、head SHA、Checks 摘要、合并/关闭、`last_synced_at`、stale
- [ ] 12.3/12.7.4 规则优先修复包 + 引文校验 + fail-open 降级；原始证据按需读取，摘要模型非默认
- [ ] 同批次验证幂等；恢复记录保存阻塞/解决证据/阶段/授权，累计预算不重置
- [ ] 19.3 调用用途、token/cache 分项、阶段耗时、验证去重指标；12.7.5 的 0a 故障场景通过
- [ ] 8.4 上下文降级路径（摘要失败 / 文件超限 / 预算不足各自可用且可追溯）
- [ ] 8.5 归档/分页：摘要与身份一致、失败显式降级、收据不二次压缩、清理后有效引用仍可读
- [ ] 12.7.6 输出视图单项基线对照，记录 recall 成本和所有失败；收益不足不开默认压缩
- [ ] 最简 Web：需求表单、列表/详情、Run 时间线、待办箱
- [ ] 错误码子集 + 执行阶段自动重试白名单 + 每阶段预算 2；未知错误 fail-closed
- [ ] 结构化日志、`agent_events`、`requirement_events`
- [ ] **验收**：5 条真实需求 ≥3 条零介入到 PR；介入原因全属 0.1 类型；声明前/后、push 后重启恢复正确

### Phase 0b：远程可用

- [ ] Cloudflare Tunnel + Access；Axum 验证 JWT（签名/issuer/aud/过期/subject）；源站只接受 Tunnel
- [ ] CSRF、`no-store`、会话到期与撤销实测、写请求不自动重放
- [ ] 手机响应式：需求表单、待办箱、Run 详情、提问回复、Stop / Retry
- [ ] 一个真实通知渠道：待办箱新增、执行失败、PR 已创建；链接不带令牌；独立重试
- [ ] 阻塞通知包含脱敏原因与是否需操作；手机同一详情页完成查看、决策与验证状态回显
- [ ] 后台不依赖浏览器：关闭页面、锁屏不影响 Run
- [ ] 多设备并发处理同一待办：第一个 CAS 胜出，其余显示已处理
- [ ] WorkspaceReconciler + 最小 GitHubReconciler
- [ ] **验收**：手机真实域名完成 0a 路径；Agent 提问时手机收到通知并回答；伪造头/错误 AUD/直连源站被拒

### Phase 0c：加固

- [ ] 低权限 executor：独立 UID/容器、cgroup 执行组整体终止；实测不能读平台凭证、其他 workspace
- [ ] Codex 模型认证与工具执行视图隔离，按 17.3 实测
- [ ] 8.7 只读诊断 Agent 隔离、自动触发条件、每 blocker 1 次/每 root 2 次预算与全局容量守卫
- [ ] 无效引文/自造动作/模型失败/预算耗尽/取消迟到降级正确，不覆盖原 blocker、不擅自恢复
- [ ] 受信 Gate policy revision 批准/撤销；Agent 改 `.harness-gate/`、hooks、验证入口只作待批准变更
- [ ] 本项目 dogfood 切换 `gate_provider = harness-gate`；`hook` + `verify --profile ci --all`；12.6.4 fail-closed
- [ ] 完整租约：Repository dispatch lease、dispatch_epoch、worktree_leases、恢复屏障、旧 epoch 只归档
- [ ] ProcessReconciler 9 条冷启动规则（5.1）
- [ ] `context_input_digest` + `NeedsRevalidation`；Contract 未变自动重验证，失败才进待办箱
- [ ] 0a 已有等待时限与 0c 完整执行组隔离联动通过
- [ ] 每日备份、升级前备份、一次隔离环境真实恢复（外部写默认禁用）；扩展 0a 已有磁盘保护
- [ ] 22.10 全部 Phase 0 行 + 22.11 部署验收通过
- [x] S5 allowlist 配置键已在 2026-09-11 补测并写回 16.9
- [ ] 补测 S5 尚未覆盖的 deny 优先级、子域/端口、主机共享占用和用户配置叠加；未验证能力不启用
- [x] 补测 S4 的暂存配置篡改（2026-09-11 完成，结论：能自证通过 → 平台必须比对 configuration_digest）
- [ ] 实现 12.6.1 的 configuration_digest 比对，并加一条测试：篡改 config 后必须拒绝采信
- [ ] **验收**：本项目自身一条需求经 Harness-Gate 全流程到 PR，零介入

### Phase 1：自动闭环（L2）

- [ ] 状态机补齐 `Queued / WaitingCI / WaitingReview / Merging / Done / Blocked`，人工 `Unblock`
- [ ] GitHubReconciler 业务对账：精确 head SHA 的必需 Checks 驱动 `WaitingCI`；缺失/skipped 不算通过
- [ ] 自动 Merge 默认开启：17.5 条件全部机械判定；MergeOperation 记录；head 变化放弃重等；未知结果先查事实
- [ ] `post_merge` 验证批次：隔离检出 merged SHA，automated_test / gate_check / ci_check 自动写入 → `Done`
- [ ] post_merge 验证失败 → revert 建议进待办箱，不回滚
- [ ] 自动 CiRecovery：必需 Check 失败 → Failure Summary → 复用原分支；预算默认 3，Policy 可配
- [ ] 自动 ReviewFix：未 resolved thread 聚合、stale 过滤、CI 通过后运行、thread 下回复处理位置
- [ ] 预算耗尽 → `Blocked` → 待办箱；Unblock 不重置计数
- [ ] RepairAttempt 覆盖声明后/CI/Review/formatter；预留与动作原子关联，重复来源不重复扣费
- [ ] formatter 新 candidate 不要求 Agent 声明；来源链/唯一约束/新 SHA 验证与交接通过
- [ ] Review 已处理版本只等待确认，重复轮询/回复丢失/head 变化不再启动 Agent
- [ ] 安全类失败（secret / 架构 / 配置 / 策略撤销）永不自动修复
- [ ] 12.7.3 provider 证据复用资格/完整身份校验；不支持时完整执行，身份/时效/撤销变化不误复用
- [ ] 12.7.2 受控 formatter 新 SHA 重验证；12.7.4 重复失败停止；12.7.5 Phase 1 场景通过
- [ ] 同 Repository `depends_on`：上游 Done 且 merged SHA 为下游基线祖先才领取
- [ ] `OutputIntegrated` 来源分类，本次 PR 改 WORKFLOW/知识文件不自触发
- [ ] 自动模式默认值正确；可选开关仅在对应能力实现后允许开启：`auto_merge=true`、`gate_recovery_policy.mode=auto`、
      `require_fix_confirmation=false`、`allow_manual_review=false`、`require_manual_start=false`
- [ ] 实现可选人工 Merge 入口时与自动路径共用校验；未实现时可在 GitHub 人工合并
- [ ] 22.10 全部 Phase 1 行通过
- [ ] **验收**：disposable 仓库 10 条需求 ≥7 条零介入到 Done（含 ≥2 条经历 CI 失败自动修复）；
      介入原因全属 0.1 类型；预算耗尽后不再消耗模型

### 可选模式（不进 V1 门槛，Policy 开启时按对应章节验收）

- [ ] 平台 CLI + Unix socket peer UID 认证（16.8、17.6）
- [ ] `allow_manual_review` 下的手机逐项人工验收页与 review_tree_equivalence 继承（7.6）
- [ ] `require_fix_confirmation` 下的修复前确认
- [ ] `require_manual_start` 下的 Ready 与 Start 分离
- [ ] 自动 reviewer Agent（Phase 1 P1）
- [ ] Webhook / SSE（Phase 1 P1 / 0b P2）
- [ ] 12.7.6 Action Fusion（Phase 1 P1）：编辑/检查结果分离、重放幂等、不代替 required 检查，单项和组合对照

### 推迟项（不进 V1，保留回归路径）

- [ ] 描述→Contract 草稿、澄清 Agent（Phase 2）
- [ ] 多 Project/Repository 并行、完整输入 digest、DigestReconciler、多仓资源协调与历史数据 GC（Phase 2）
- [ ] 8.6 主动上下文压缩（Phase 2 可选）：成本/窗口压力分开、保留恢复锚点、取消/阻塞不自动续跑
- [ ] Planner / Discussion / 公平调度 / 事件溯源（Phase 3）
- [ ] 多 Runtime / fallback / 金额预算 / Remote Worker / PWA / 跨仓 Requirement / 自动部署（Phase 4）
- [ ] 原生 Codex TUI/Remote 接入与独立会话接管（后续独立实验，不作为已支持能力）

---

## 25. 设计约束索引

本节只提供规则入口；实现遇到边界问题时以对应章节为准。

| 约束 | 权威章节 |
|---|---|
| 领域真相、完成声明、封存与交接 | 3～4 |
| 旧执行组隔离、租约与陈旧结果拒绝 | 5～6 |
| Contract、依赖、输入来源及最终验收 | 7 |
| Codex 协议、受控提交、审批与人工输入 | 8 |
| 配置快照、Git 写入与资源生命周期 | 9～10 |
| GitHub 事实、受信 Gate、Recovery / ReviewFix | 11～13 |
| 追加式证据、事务、outbox 与取消先后语义 | 14 |
| 异常待办箱、多端决策幂等、可选 CLI、通知与断线返回 | 8.1、16.8、17.6、18～19 |
| Cloudflare Access/Tunnel、管理 API 与部署隔离 | 17、21.4 |
| 单机备份、恢复与磁盘保护 | 21.5、22.11 |
| 故障窗口、阶段范围与 V1 验收、介入率指标 | 22～24 |
| 错误分类、阶段动作与重试预算 | 28 |

---

## 26. 主要风险

### 风险 1：Agent 自动修改范围过大

缓解：

- 独立 workspace
- 独立 branch
- GitHub branch protection（平台不在 bypass 列表）
- 执行前后 diff 检查
- 最小权限
- 自动 Merge 的条件全部机械判定，Agent 不能触发合并或修改条件；Policy 可关闭进入信任建立期

### 风险 2：Webhook 重复或乱序

缓解：

- delivery_id 唯一约束
- inbox/outbox
- 状态版本号
- Reconciler 定期对账

### 风险 3：修复循环消耗大量模型额度

缓解：

- 根需求修复预算默认 3，Policy 可配置；耗尽进入 Blocked，不再调用模型
- 执行/验证/交接分别限次，交接失败不再调用模型
- Phase 4 增加金额预算；前期先采集成本
- Blocked 状态 + 人工 Unblock，Unblock 不重置计数

### 风险 4：多项目资源互相饥饿

缓解：

- MVP 同 Repository 串行 + 全局并发上限
- Phase 3 恢复 Project queue 与 weighted round-robin
- project/repository capacity（Phase 3）
- 独立指标

### 风险 5：Workflow 配置污染其他 Repository

缓解：

- Repository 级 Workflow
- Run 时保存 workflow hash
- 不使用全局 Config 单例
- 受信 policy 独立批准，Agent 产出不能自行激活新 hooks 或验证入口

### 风险 6：首版长期停留在执行内核，第一条需求迟迟跑不通

缓解：

- 按第 23 章 0a → 0b → 0c 切片交付，0a 只做 localhost 闭环，不按架构章节逐个建设后台模块；
- Cloudflare、通知、完整 OS 隔离、Harness-Gate 必选、备份后置到 0b/0c；0a 保留 custom 验证与最小恢复，不阻塞第一条需求；
- 多仓并行、Planner、自建审查器、完整 PWA、CLI 不阻塞任何 Phase；
- 实施前先做 S1～S5 spike，避免在不成立的协议假设上建整个流程。

### 风险 7：手机断线、通知失败或本机离线导致错过决策

缓解：

- 持久化待办、独立人工等待时限、会话重登恢复和多端幂等；
- 通知故障独立显示，不默认批准或自动重启 Agent；
- 开机启动、备份恢复、磁盘保护；明确主机断电期间平台不可用；
- 低频/失效同步显示时间与错误，不让陈旧的绿色状态误导合并决策；
- 正常路径零介入意味着错过决策的场景本身就少：只有 Agent 提问和异常才需要人。

### 风险 8：自动合并把"实现了错误需求"的代码合进主干

这是零介入路线特有的风险：AC 验证通过只证明"做了 AC 说的事"，不证明"AC 说的就是你要的"。

缓解：

- 把人的判断前移到评审：评审对象是结构化 Contract 和逐条 AC，不是自然语言描述；
- 信任建立期：Policy 关闭自动 Merge，前 N 条需求人看 PR 再合，观察 AC 验证与自己判断的一致率；
- post_merge 验证失败自动生成 revert 建议，个人仓库 revert 成本低；
- 合并 commit 附 Requirement 追溯标记，任何一次合并都能回到它的 Contract 和评审记录；
- Phase 2 的澄清 Agent 在评审前挑出歧义和不可验证的 AC，降低"评审时没想到"的概率。

### 风险 9：需求质量成为新瓶颈

零介入之后，系统产出的上限就是 Contract 的质量；前几十条需求大概会发现自己写的 AC 不够精确。

缓解：

- 接受校准期：0a 验收只要求 5 条里 3 条零介入，Phase 1 要求 10 条里 7 条；
- Agent 提问是正常路径上唯一预期的介入，把它当作"AC 不够好"的反馈信号收集，
  统计提问原因，反哺 Contract 模板和 AC 写法；
- Phase 2 的描述→Contract 草稿 + 澄清 Agent 把"写 AC"变成"改 AC"；
- 不要用放宽自动化来补需求质量：宁可 Agent 多问一次，也不让它猜。

---

## 27. 最终产品形态

```text
                       Personal AI Software Factory

Personal Account
       │
       ├── Project A
       │     ├── Repository A1
       │     └── Repository A2
       │
       ├── Project B
       │     ├── Repository B1
       │     └── Repository B2
       │
       └── Project C
             └── Repository C1

Repository Knowledge（可选 Git 文件）
    ↓
Discussion（可选）
    ↓
Requirement Contract
    ↓
Requirements
    ↓
Global Scheduler
    ↓
Project / Repository Capacity
    ↓
AgentRun
    ↓
Codex Runtime
    ↓
Workspace / Worktree
    ↓
Commit / Push
    ↓
GitHub PR
    ├── CI Passed
    ├── Review Approved
    └── Merge
            ↓
           Done

CI Failed
    ↓
CiRecovery Requirement
    ↓
原 PR Branch

Review Comment
    ↓
ReviewFix Requirement
    ↓
原 PR Branch
```

最终定义：

> 一个面向个人开发者的、以需求评审为唯一人工决策点、以 Requirement 为核心、以 Agent 为执行单元、
> 以 GitHub 为协作基础设施的 AI 软件研发自动化平台：评审通过的需求在正常路径上零介入地变成已合并代码。

它继承 Symphony 的执行内核，但将单项目参考实现升级为：

```text
多项目配置（数据模型）
多仓库调度（Phase 2 起）
全局公平容量管理（Phase 3 起）
持久化状态
PR/CI/Review 轮询对账（Phase 0 起；Webhook 后续可选加速）
可恢复 Agent Run
自动合并与自动 Done（Phase 1 起）
自动 CI / Review 修复（Phase 1 起）
```

交付路径以 Phase 0a 本机闭环为起点（第 23 章），上述能力按 Phase 渐进恢复，不在 V1 一次性铺开。
Phase 0 交付"评审后自动到 PR"，人在 GitHub 合并；Phase 1 交付"评审后自动到 Done"；
图中的公平容量、Discussion 不代表 V1 已实现。

---

## 28. 错误码表与重试策略

所有 `failure_code` 统一映射 `retry_class`，由 RetryPolicy 引擎消费。

策略优先级：

```text
安全/状态机硬规则
    > failure_phase 对应的恢复路径
    > Repository / Project Policy
    > Requirement.retry_policy 覆盖
    > 错误码表默认 max_retries / backoff
```

Requirement 覆盖不能把 manual blocker 改成自动重试，也不能超过阶段硬上限。
blocked 先表示停止当前执行；只有 8.1 的平台白名单分类允许有限 auto_probe，解除证据落库并
通过全部恢复守卫后才能继续。探测不是重试编码；错误码自身不能授权解除。

错误记录必须带 failure_phase，不能仅凭 retry_class 决定是否启动新的 AgentRun。
未解除的持久化 blocker、未确认静止的旧组、未保全的工作或控制面存储故障优先阻止重试；
agent_process_lost/stalled 等白名单不能覆盖这些条件：

| failure_phase | 自动恢复动作 | 等待状态与计数 |
|---|---|---|
| preparation | 只重试环境准备/预检，模型尚未启动 | 领取前为不可领取原因；已领取则保持准备阶段，独立 preparation_attempt |
| execution | 先检查未解除 blocker；停止旧组，无声明/manifest 时核查工作快照与 Recovery Context 后新 Run 续作 | Ready + not_before_at；execution_attempt |
| validation | 重试固定 candidate SHA 的受信验证，不重启模型 | Requirement 保持 Running 的验证子阶段；原 Run 终态不变，verification_attempt |
| handoff | 对账并重试同一 push/PR 步骤 | Running + Handoff RetryWaiting；operation_attempt |
| post_merge | 等待精确 SHA 的 CI 或重试平台验证 | WaitingCI/WaitingReview；verification_attempt |

`preparation` 是失败/计费阶段，不新增业务状态。领取前资源不足仍不创建 Run；已创建的
Run 尚未启动模型时，准备故障单独记录，耗尽按 Phase 0 的 Failed + 待办处理。
准备阶段只对白名单内临时 git_fetch_failed / git_worktree_create_failed / preparation_timeout
重试，Phase 0 同一 Requirement + generation 最多 2 次自动重试，另设 deadline；确定性
缺依赖/权限/能力问题不反复执行。下列 Git 错误发生在模型启动前时归 preparation，
不得因此启动新的编码会话或扣 execution_attempt。

MVP 的执行阶段自动重试白名单：

```text
agent_startup_timeout
agent_response_timeout
agent_stalled
agent_process_lost
git_fetch_failed
git_worktree_create_failed
git_dirty_workspace
agent_protocol_error
model_rate_limit
model_unavailable
model_invalid_response
```

MVP 验证阶段仅允许 gate_timeout 等明确的基础设施超时重试；交接阶段允许 git_push_failed、
github_api_rate_limit、github_api_error 和可确认临时的 pr_create_failed。GitHub 认证/权限错误、
远端 head 冲突不进入临时重推；git_push_rejected 先只读对账，只有 14.4 已授权的新候选修复
可继续，不自动 rebase。

Phase 0 不创建 CI Recovery/ReviewFix。完成声明前 hook 的 test/lint/build 失败按 12.6.4
反馈给当前 Agent，不能视为新的 Run 重试；声明后的 fixable 失败（test/lint/build/AC）在执行预算内
自动创建一次带失败摘要的新 Run 重新编码，仍失败才转人工。

Phase 0 的 RetryPolicy 只允许以下自动重试：

```text
transient / recoverable 且属于当前阶段 allowlist → 在同一阶段有限重试
声明后的 fixable                                → 执行预算内一次带失败摘要的重新编码 Run
blocked 且平台分类为 auto_probe                 → Failed + manual_action_required=false，有限探测后按计划恢复
其他 blocked / 未知错误                         → Failed + manual_action_required=true
```

Phase 0 执行/准备/验证/交接各阶段自动重试总预算最多 2 次，单错误上限取 min(表中默认值, 2)；执行预算按
Requirement + generation 汇总，准备预算独立按 Requirement + generation 汇总，验证预算按 candidate + policy + stage 的恢复链汇总（batch 保存引用），交接预算按 candidate + branch
的交接恢复链汇总（operation 保存引用）。
8.1 的只读 blocker 探测默认最多 6 次，10.6 清理重试默认最多 3 次，各用独立计数；
它们不属于这里的执行阶段 2 次预算，不据此增加编码或准备动作次数。创建新批次/重投操作不得重置同一失败链预算。
不同错误交替出现不能重置预算，自动重试不能通过递增 generation 绕过限制。
人工重新授权可以开启新轮次，但保留累计历史。退避使用指数增长和 jitter，429 优先遵守
服务端重置时间；所有阶段还有明确 deadline。执行的有效工作时长、独立人工等待时限和
Run 绝对寿命按 8.1 分开计算，不能让普通 turn/stall deadline 提前误杀待审批任务。
Phase 1+ 放宽基础设施重试次数必须显式配置；不能放宽 12.5 的阶段修复硬上限。

人工等待时的失联、进程重启或批准送达未知优先执行 8.1/5.1 的失效与人工恢复规则，
即使底层错误为 agent_process_lost/agent_response_timeout，也不能自动重启绕过待定决策。
通知失败、只读 GitHub 同步失败和拒绝的 HTTP 鉴权请求分别记录在对应服务/请求中，
不套用执行失败状态机，不把正常 Run 误改 Failed，也不消耗执行重试预算。

### retry_class

```text
transient   临时失败，自动重试（指数退避 + jitter）
recoverable 平台安全清理、停止后重启或对账重新投递，可在原阶段有限重试
fixable     需修改代码；先尝试白名单确定性修复，必要时交给 Agent，统一受 RepairAttempt 上限约束
blocked     停止当前执行；按平台分类有限探测环境，或请求人工；不直接自动重试编码
```

### Git

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| git_fetch_failed | transient | 3 | 指数退避重试（5s/15s/45s） |
| git_auth_failed | blocked | 0 | 检查 GitHub App 凭证 |
| git_worktree_create_failed | recoverable | 2 | 确认旧组静止并归档后重建 worktree |
| git_branch_conflict | blocked | 0 | 人工解决分支冲突 |
| git_dirty_workspace | recoverable | 1 | 保存现场后重建；声明后发现脏工作区不得自动丢弃变更 |
| git_push_failed | transient | 2 | 对账远端，仅重试固定 SHA 的交接步骤 |
| git_push_rejected | recoverable | 0 | 禁止盲目重推；有限只读对账后按 14.4 决定幂等成功、授权内新修复或人工，保护规则拒绝不绕过 |
| git_rebase_conflict | blocked | 0 | 人工解决冲突 |

### 环境准备（不启动模型）

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| preparation_timeout | transient | 2 | 只重试固定准备步骤，遵守准备总预算与 deadline |
| preparation_dependency_missing | blocked | 0 | 已授权固定输入可由平台预置并有限探测；需要扩大授权或探测耗尽才待办 |
| preparation_capability_mismatch | blocked | 0 | 已批准工具预置流程可自动准备并重查；无法按既定流程解决则待办 |
| preparation_permission_denied | blocked | 0 | 修复执行环境权限，不放宽沙箱、不调用模型绕过 |

### Harness-Gate

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| gate_config_invalid | blocked | 0 | 修复 `.harness-gate/` 配置 |
| gate_environment_invalid | blocked | 0 | 修复 doctor 检查项 |
| gate_policy_unapproved | blocked | 0 | 人工批准受信策略，当前产出不得自证通过 |
| gate_policy_revoked | blocked | 0 | 停止后续调用，在新策略下重新验证 |
| gate_evidence_invalid | blocked | 0 | 证据缺失/篡改，保留现场并人工处理 |
| gate_secret_detected_high | blocked | 0 | 移除凭证后人工审查 |
| gate_secret_detected_low | blocked | 0 | 移除疑似凭证后人工审查 |
| gate_architecture_violation | blocked | 0 | 人工审查架构变更 |
| gate_test_failed | fixable | 3 | CiRecovery 修复测试 |
| gate_lint_failed | fixable | 2 | CiRecovery 修复 lint |
| gate_build_failed | fixable | 3 | CiRecovery 修复编译 |
| gate_timeout | transient | 2 | 只重试同一 SHA 的验证步骤，不重启 Agent |

Gate 的阶段动作以 12.6.4 为准。`gate_hook_failed` 只是完成声明前的反馈封装，必须保留底层
failure code；只有 test/lint/build 可有限反馈修复，config/secret/architecture/evidence 类立即
转人工，不能被统一封装绕过。达到 turn 上限按 agent_max_turns_exceeded 处理。

### Agent Runtime

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| agent_startup_timeout | transient | 2 | 自动重试 |
| agent_response_timeout | transient | 1 | 终止并确认旧组退出；按声明状态恢复，禁止盲目重发当前 turn |
| agent_turn_timeout | blocked | 0 | 人工审查，可能需要调整 Requirement |
| agent_stalled | transient | 2 | 排除有效人工等待；停止完整执行组并确认静止后按声明状态恢复 |
| agent_protocol_error | recoverable | 2 | 停止旧执行组后按声明状态恢复，不复用不确定会话 |
| agent_process_lost | transient | 2 | 确认旧组静止；先核查 blocker/快照/恢复上下文，有声明只恢复平台步骤 |
| agent_completion_not_reported | blocked | 0 | 未声明完成，保全工作快照；不能交接中间 commit |
| agent_blocker_reported | blocked | 0 | 停止续轮、保全工作；按 8.1 auto_probe/manual 分类，解除后按阶段恢复 |
| recovery_state_invalid | blocked | 0 | 必需快照/恢复记录缺失、损坏或身份不匹配；保留原目录，不从干净基线自动重跑 |
| agent_approval_timeout | blocked | 0 | 请求过期，停止执行并转人工 |
| agent_input_timeout | blocked | 0 | 人工输入过期，停止执行并转人工 |
| agent_request_invalidated | blocked | 0 | 重启/失联后的旧请求失效，转人工，不复用审批 |
| agent_run_lifetime_exceeded | blocked | 0 | 达到独立 Run 绝对寿命上限，停止执行组并转人工 |
| agent_approval_denied | blocked | 0 | 人工决策或调整审批策略 |
| network_domain_not_allowed | blocked | 0 | 仅无法继续时终止并保全；需扩大范围则聚合一次 Contract 修订待办，不消耗修复预算 |
| agent_max_turns_exceeded | blocked | 0 | 拆分 Requirement |
| agent_repeated_failure | blocked | 0 | Phase 1：12.7.4 同输入确定性失败达到阈值；停止重复恢复，保留修复包与累计预算 |

### Model Provider

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| model_rate_limit | transient | 5 | 指数退避（10s/30s/90s/270s/810s） |
| model_unavailable | transient | 3 | MVP 按执行预算重试；模型 fallback 从 Phase 4 启用 |
| model_context_length_exceeded | blocked | 0 | 减少 Agent Context 大小 |
| model_auth_failed | blocked | 0 | 检查模型凭证 |
| model_invalid_response | recoverable | 2 | 自动重试 |

### CI / GitHub

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| ci_failed | fixable | 3 | CiRecovery Requirement |
| github_api_rate_limit | transient | 5 | 等待速率限制重置 |
| github_api_error | transient | 3 | 仅对网络/5xx 等临时失败对账并重试原步骤 |
| github_permission_denied | blocked | 0 | 人工修复安装权限，不能通过重新编码恢复 |
| pr_create_failed | recoverable | 2 | 查已有 PR 和分支；422 "already exists" 走查找路径不算失败 |
| pr_merge_conflict | blocked | 0 | 人工解决合并冲突 |
| github_merge_head_moved | recoverable | 0 | `PUT /merge` 带 sha 返回 409；放弃本次 MergeOperation，按新 head 重新等待条件 |

表中 fixable 的 max_retries 是单错误码的上界，实际次数还受根需求修复预算（12.5，默认 3，
Policy 可配置）约束，取两者中更严格的；Phase 0 只展示外部 CI 失败，不创建 CiRecovery。

### 控制台与配套服务（不触发编码重试）

| code | 记录位置 | 动作 |
|---|---|---|
| access_unauthorized | API 安全审计 | 拒绝请求/要求重新登录，不改变 Run |
| runtime_request_conflict | 审批请求审计 | 返回冲突或失效状态，刷新待办，不重放决策 |
| notification_delivery_failed | notification_deliveries | 通知自身有限重试、告警，不重启编码 |
| github_sync_stale | GitHub 同步状态 | 限流退避，显示最后成功时间，不伪造 CI/PR 结果 |
| host_disk_low | 控制面健康/调度原因 | 暂停领取，已授权 ResourceCleaner 回收并有限探测；持续不足才运维待办，活动工作按 3.7 保全 |
| control_storage_unavailable | 控制面健康/恢复记录 | 停止新动作和活动写入组，保留原目录；恢复后先保全/对账，不能以无声明触发重跑 |
| backup_failed | 备份记录/待办 | 保留上一份可用副本，提示人工处理 |
| resource_cleanup_failed | cleanup_tasks / 运维待办 | 清理自身有限重试/人工修复；不新建 AgentRun，不更改业务完成事实 |
| diagnostic_bundle_incomplete | diagnostic_bundles | 展示缺失证据与采集失败原因，保留已知事实；不使原 blocker 消失 |
| diagnostic_unavailable / diagnostic_budget_exhausted | diagnostic_attempts / blocker 详情 | 固定诊断和具体下一步仍可用；不自动追加模型调用或扣代码修复预算 |
| diagnostic_output_invalid | diagnosis_reports / 诊断审计 | 拒绝无证据结论与自造动作，退回固定诊断，不覆盖原失败码 |

### Business Logic

| code | retry_class | max_retries | recovery_action |
|---|---|---|---|
| requirement_invalid_state | blocked | 0 | 拒绝非法命令，不使合法的当前执行失败 |
| requirement_digest_mismatch | blocked | 0 | 按 7.6 进入 NeedsRevalidation，而不是普通失败重试 |
| execution_lease_lost | blocked | 0 | 停止完整执行组，隔离旧 epoch，确认静止后人工恢复 |
| execution_group_unquiesced | blocked | 0 | 保留恢复屏障与容量，禁止接管 |
| artifact_manifest_invalid | blocked | 0 | 声明/HEAD/封存证据不一致，拒绝交接 |
| verification_evidence_missing | blocked | 0 | 最终 SHA 证据等待超时，人工补充，不能 Done |
| max_recovery_attempts_exceeded | blocked | 0 | 人工审查和 Unblock |

容量不足、依赖未满足和领取前的 workspace lease 冲突不是执行失败：Requirement 尚未满足
调度条件时不创建 Run、不消耗 retry 次数，只作为 Scheduler 的不可领取原因和指标记录。
退避和资源释放按本章阶段表及 6.3 执行；不能把交接等待套用成 Ready + 新 AgentRun。
取消、输入替代和迟到消息属于授权/状态机处理，不通过通用错误处理器覆盖已有业务状态。

未知 `failure_code` / `retry_class` 不允许 auto_probe，一律 fail-closed：MVP 记为
`Failed + manual_action_required=true`；Phase 1+ 记为 `Blocked`。

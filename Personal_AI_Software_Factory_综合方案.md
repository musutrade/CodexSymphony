# Personal AI Software Factory 综合方案

> 面向个人开发者的多项目、多仓库、多 Agent 软件研发自动化平台
> 版本：V7 Draft
> 日期：2026-09-05
> 状态：设计规格；实施范围和验收条件以第 22～24 章为准

---

## 0. 结论摘要

本方案吸收 OpenAI Symphony 的核心设计，但不把 Symphony 当前“单进程、单 Tracker Scope”的参考实现直接当作平台边界。

目标系统必须支持：

```text
一个个人账户
    ↓
多个 Project
    ↓
每个 Project 多个 GitHub Repository
    ↓
多个 Agent 跨 Project / Repository 并发运行
```

其中：

- 平台可以同时处理多个 Project 和多个 Repository。
- 一个 Requirement 在 Phase 1 绑定一个 Repository。
- 同一个 Requirement 同时修改多个 Repository，留到后续版本。
- 一个 Agent Runtime 可以同时运行多个 Agent Run。
- CI Recovery 和 ReviewFix 默认复用原 PR 分支，不创建无关的新 PR。
- 本项目 Angular 界面遵循关联 `arc-admin` 仓库的语义 Token、页面结构、响应式、无障碍和视觉验收规范；该规范只约束本项目自身前端，不强制外部 Repository。
- Personal AI Software Factory 自身仓库的提交、PR、CI、发布和平台 Agent 自检都必须经过 Harness-Gate；门禁失败时必须 fail closed。
- 平台管理的外部 Repository 是否采用 Harness-Gate，由各 Repository Policy 决定；不得把本项目的门禁配置隐式复制成所有仓库的全局配置。
- ADR/OpenSpec 是可选仓库文件，不是平台一级实体；执行和验收只依赖 Requirement Contract 与带稳定 ID 的 AC。

最重要的工程原则是：

```text
Requirement 状态是平台唯一业务真相
AgentRun 状态是执行事实
GitHub / CI / Review 是外部观察事实
```

V1 采用 MVP 先行策略：

- 数据模型保留多 Project、多 Repository，但 MVP 只用一个 Project / 一个 Repository 跑最小闭环；
- MVP（Phase 0）状态机核心 5 态：`Draft → Ready → Running → Submitted / Failed`；`Submitted` 表示 PR 已创建并交接给 GitHub；
- 调度：全局并发 4 + 同 Repository 串行 + FIFO（priority 排序），公平调度与 Project 容量推迟；
- `context_input_digest`（Contract + AC + WORKFLOW.md + 实际读取知识文件的
  path + blob_oid；本地未提交例外为 content_hash + source）进入 MVP；
- CodexRuntime 直接实现，参考 Symphony 的 `codex/app_server.ex`，协议类型用 schema codegen。
- Agent 负责 workspace 内修改，并通过受控 `create_local_commit` 工具请求本地提交；
  工具不授予通用 Git 写权限或 GitHub 凭证；
- Agent 必须通过 `report_completion` 显式声明完成；平台持久化声明、停止写入、验证并封存产出；
- push、PR 创建/更新通过独立的 HandoffOperation 执行；交接重试不重新启动 Agent；
- MVP 只支持信息性依赖；阻塞调度的执行性依赖从 Phase 1 启用；
- Gate 配置来自受信快照，仓库 hooks/test 与平台凭证隔离；产出不能自行改变验证规则。

本文的分期术语固定为：

```text
MVP / Phase 0 = 单 Project、单 Repository，完成 PR 创建并进入 Submitted
V1 = Phase 0 + Phase 1，增加 CI、Review、Webhook、手动 Merge 后的完整 Done 闭环
Phase 2        = CI Recovery、ReviewFix 和完整输入对账
```

本文不把“V1”当作独立实施阶段名称；V1 只是 `Phase 0 + Phase 1` 的交付标签，所有实施
范围以 `Phase 0`、`Phase 1`、`Phase 2` 为准。

平台的第一目标不是“做一个很大的 AI 管理后台”，而是稳定完成：

```text
Requirement
    → Agent
    → Worktree
    → 本项目 Harness-Gate Hook
    → Local Commit
    → Platform Push / PR（GitHub Adapter）
    → Pull Request
    → Submitted（MVP）
    → GitHub Actions + 本项目 Harness-Gate CI
    → Review
    → Done（Phase 1）
```

上图是 Phase 1 之后的完整目标路径；MVP 在 `Pull Request → Submitted` 交接处结束。
MVP 的 `Submitted` 判定由平台完成：Agent 只产生本地 commit，GitHub Adapter 通过 outbox
执行 push、查找/创建 PR，并在 PR 关联持久化后推进状态。

失败和评审意见进入同一个可审计的修复闭环：

```text
CI Failure / Review Comment
    → Recovery Requirement / ReviewFix Requirement
    → 原 PR 分支
    → Agent
    → CI / Review
```

---

## 1. 产品定位

### 1.1 一句话定义

Personal AI Software Factory 是一个以 Requirement 为核心、以 GitHub 为代码协作基础设施、以 Codex Agent 为第一执行引擎的个人软件工厂。

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

Phase 1 中每个 Requirement 必须绑定一个 Repository：

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
recovery_source_type = ci_failure | review_comment | null
recovery_source_id
```

非 Recovery Requirement 的两个字段必须为 `null`。同一来源只允许创建一个对应的修复
Requirement（见 14.3）。

`root_requirement_id` 统一使用这一命名：根 Requirement 的值为自身 ID，Recovery /
ReviewFix 子 Requirement 指向根 Requirement。`total_recovery_attempts` 只在根
Requirement 上维护，子任务读取根任务的聚合值。

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
  运行受信 Gate，封存 commit 和不可变产出清单；
  通过 outbox 幂等执行 push；
  按 repository + branch 查找或创建 PR；
  只有 PR 已创建且关联已持久化，才允许 Requirement 进入 Submitted。
```

### 3.5 完成声明、封存产出与交接

三个记录各有独立职责，不能用“发现一个 commit”替代：

| 记录 | 必备绑定 | 意义 |
|---|---|---|
| CompletionDeclaration | run_id、generation、revision_id、context_input_digest、声明时 HEAD、summary、received_at | Agent 明确请求结束；先持久化再回复工具成功 |
| ArtifactManifest | declaration_id、artifact SHA/tree、change_base SHA、snapshot_id、Gate policy/result、sealed_ref、sealed_at | 平台确认写入静止，验证通过并保存在 Agent 不可写的位置 |
| HandoffOperation | requirement_id、run_id、generation、manifest_id、verification_batch_id、input_authorization_event_id、repository/branch、expected_remote_sha、operation_attempt | 平台对固定产出的 push/PR 交接；重验证时保留显式授权引用 |

CompletionDeclaration 与 ArtifactManifest 不可变；每个 Run 最多一个被接受的完成声明和一个
有效封存产出。重复工具调用返回原结果；参数不同的重复声明拒绝。新 HEAD 或 Contract/AC
变化需要新 Run，不得修改已有声明指向的 SHA。其他输入变化能否复用原产出，按 7.6 的人工
重验证规则决定，不回写历史输入。

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

---

## 4. 统一状态真相

### 4.1 Requirement 状态

先按实施阶段阅读状态：

| 阶段 | 启用状态 | 成功终点 |
|---|---|---|
| MVP / Phase 0 | `Draft`、`Ready`、`Running`、`Submitted`、`Failed`、`Cancelled`、`NeedsRevalidation` | `Submitted` |
| Phase 1 | 增加 `Queued`、`WaitingCI`、`WaitingReview`、`Done`、`Blocked` 和人工 `Unblock`；`Merging` 仅自动 Merge 开启时使用 | `Done` |
| Phase 2 | 增加 Recovery / ReviewFix，复用已有 `Blocked` / `Unblock` | 根任务恢复或 `Blocked` |

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

Phase 1 默认人工 Merge。`PullRequestMerged` 只记录合并事实并启动最终验证；只有 7.6 的
全部完成条件满足，才进入 `Done`。合并后的自动验证保留 `WaitingCI`，缺少人工证据时保留
`WaitingReview`，通过 `completion_phase=post_merge_validation` 显示子阶段。
`Merging` 仅用于未来启用自动 Merge 的实现。

异常转换：

```text
Running
  → Failed

Failed
  → Ready              人工重试 Agent 执行，保留历史 Run
  → Running            人工重试仍有效的封存产出交接，不启动 Agent

Cancelled
  → Draft              人工重新打开

Running / Submitted / WaitingCI / WaitingReview
  → Blocked            Phase 1+
  → 原受阻阶段         人工 Unblock，重新校验后继续；不盲目重新编码

Ready / Queued / Running / Submitted / WaitingCI / WaitingReview
  → NeedsRevalidation
  → Ready              需要新执行
  → Running / Submitted / WaitingCI / WaitingReview
                       有有效产出且重验证通过，恢复原阶段

任意未完成状态
  → Cancelled
```

`Done` 的定义必须明确：

```text
Phase 1 默认：
Requirement 的 PR 已合并，且 CI、验收标准和必要 Review 均通过。
```

`Submitted` 是平台的代码交接态：

```text
MVP / Phase 0：
完成声明和封存产出有效，产出 commit 已 push，PR 已创建，平台已保存 PR 关联；
generation、当前输入资格、受信 Gate 与远端 head 校验全部通过
```

`Submitted` 不表示 CI、Review 或 Merge 已完成；这些事实在 Phase 1 通过 Webhook / Reconciler
驱动后续 `WaitingCI → WaitingReview → Done`。收到 `PullRequestMerged` 事件前，不能进入
`Done`。

重新执行规则：

```text
Failed → Ready：执行失败的人工 retry，默认复用仍开放的原分支和 PR；
Failed → Running：交接失败的人工 retry，只恢复有效 manifest 的交接操作；
Cancelled → Draft：人工重新打开，默认重新创建分支；
NeedsRevalidation：按 7.6 选择新执行或重新授权既有产出，不无条件回 Ready。
```

#### MVP 简化状态

MVP 只使用核心 5 态：

```text
Draft → Ready → Running → Submitted / Failed
```

另保留两个异常态：

```text
Cancelled          手动停止 / 取消
NeedsRevalidation  最小 digest 漂移（Contract / AC / WORKFLOW.md 变化）
```

`Queued`、`WaitingCI`、`WaitingReview`、`Done`、`Blocked` 从 Phase 1 启用；
`Merging` 仅未来自动 Merge 使用：
MVP 不读 CI / Review、不自动 Merge；Phase 1 恢复 GitHub 闭环后按上面的完整状态机回归。
`Submitted` 是 MVP 的终态交接，不是最终业务完成。`Running` 同时覆盖 Agent 执行和平台交接，
UI 从 active Run / HandoffOperation 派生 `execution_phase`，不得用“没有活跃进程”判定失败。

MVP 不暴露持久化 `Blocked` 状态。需要人工处理的失败统一为：

```text
Requirement.state = Failed
failure_code = ...
manual_action_required = true
```

Phase 1 启用独立的持久化 `Blocked` 状态及其人工恢复命令。

状态语义按分期固定：

| 阶段 | 成功终点 | 需要人工处理的失败 |
|---|---|---|
| MVP / Phase 0 | `Submitted` | `Failed + failure_code + manual_action_required=true` |
| Phase 1 | `Done` | `Blocked + failure_code + manual_action_required=true` |
| Phase 2 | Recovery / ReviewFix 成功后回到根任务流程 | 超过修复上限进入 `Blocked` |

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

AgentRun 在完成声明、写入静止、验证和封存成功后进入 `Succeeded`，不等待 GitHub。
AgentRun 状态不能直接决定 Requirement 状态。例如：

```text
AgentRun = Succeeded
HandoffOperation = RetryWaiting
Requirement = Running
```

终态 AgentRun 不重开。人工对既有有效声明的固定 SHA 重新验证时创建新 VerificationBatch，
保留原 Run 的失败历史；验证通过可以封存或重新授权该产出并交接，不把历史 Run 改写为成功。

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
│ Angular Web / PWA                           │
│ Requirement / Project / Agent / PR / CI     │
└──────────────────────┬──────────────────────┘
                       │ HTTP + SSE
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

上图是目标形态；MVP 只启用单 Project / 单 Repository，调度采用第 6 章的简化模型，Reconciler 只运行
Process + Workspace 两个组件（见 5.1）。MVP 的输入漂移检查由 Scheduler / Supervisor
在关键时机同步执行，不依赖独立 DigestReconciler。

### 5.1 组件职责

#### API

- 鉴权
- Project / Repository / Requirement CRUD
- 启动、停止、重试、取消命令
- Discussion 和文档接口
- GitHub webhook 接收
- SSE 订阅

#### Global Scheduler

- 扫描所有 `Ready` / `Queued` Requirement
- 检查依赖
- 检查全局容量（Phase 0 / Phase 1 默认 4）与同 Repository 串行约束
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
ProcessReconciler       每 30 秒   执行组、完成声明与交接状态对账；隔离旧写入者后恢复
WorkspaceReconciler     每 5 分钟  清理孤儿 workspace / worktree / branch
GitHubReconciler        每 1 分钟  同步 PR / CI / Review 外部状态
DigestReconciler        事件触发   处理 InputSnapshotChanged，重算 context digest
ResourceCleaner         每 1 小时  清理过期 workspace 和历史数据
```

Phase 0 只实现 `ProcessReconciler` 和 `WorkspaceReconciler`；GitHub 对账随 Phase 1
增加。MVP 的最小输入 digest 检查由 Scheduler / Supervisor 在 `Ready`、Agent start、
提交/终态判定前执行，不依赖独立 DigestReconciler。完整 DigestReconciler 与资源清理随
Phase 2 增加。

ProcessReconciler 的冷启动规则：

```text
1. 先为旧 Run、验证任务和 HandoffOperation 建立恢复屏障，暂不向对应 branch 发放新写入权；
   先检查取消和 generation 失效，失效者只允许停止/归档/对账，禁止进入后续执行恢复路径。
2. 按 execution_group_id、进程启动时间和 worker_incarnation_id 核对真实执行组；
   incarnation 不同只说明失联，不能说明已停止。终止整个执行组并确认所有子进程退出。
3. 无法确认退出或隔离成功时，保留恢复屏障和容量占用，记录人工处理原因；不得过期接管。
4. 没有持久化 CompletionDeclaration：中间 commit 只归档，不 push、不补建 PR；
   旧 Run 结束后按执行重试预算创建新的 Run，必要时复用已验证来源的中间 commit。
5. 有声明但没有 manifest：校验声明的 generation/revision/digest/HEAD，恢复平台验证和封存；
   HEAD 不同、声明失效或 Gate 不通过时拒绝交接，不以现存 PR 替代这些检查。
6. 有有效 manifest 或未完成 HandoffOperation：只恢复交接步骤，不创建新的编码 Run；
   已 push 未建 PR、PR 已建但响应丢失分别读远端事实后补齐。
7. generation 已取消或被新输入替代：禁止新的外部调用；已发出但结果未知的调用先归档对账，
   不推进 Submitted，不自动删除分支或关闭 PR。
8. Running 且没有 active Run、待封存声明、未完成验证批次或未完成交接，才是孤立状态；
   核实执行组已静止且无外部调用在途后，按原失败阶段和重试预算恢复或转人工。
9. Submitted 必须满足有效声明、manifest、当前授权/输入资格、Gate 和持久化 PR 关联；
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

---

## 6. 调度与容量（Phase 0 / Phase 1 简化）

Phase 0 / Phase 1 采用全局容量与同仓串行的简化模型。多 Project 公平容量管理推迟到 Phase 3
（见第 23 章）。

### 6.1 全局容量与 Repository 串行

调度一个 Requirement 必须同时满足：

```text
global_capacity_available（默认 4 个并发 AgentRun）
AND repository_serial_available（同一 Repository 同时最多 1 个执行/交接写入者）
AND runtime_capacity_available
AND dependency_satisfied
AND no_active_conflicting_run
AND no_unresolved_execution_group
AND no_pending_handoff_on_target_branch
```

Phase 0 / Phase 1 不实现 Project 级容量，也不做 weighted round-robin 公平调度。

Agent 容量只统计 active AgentRun；交接重试等待不占该容量。Handoff Worker 与 Agent 共用
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

Repository lease 持有者可以是 Run 或 HandoffOperation。branch reservation 使用独立的
`worktree_leases` 记录，其寿命覆盖执行、封存和交接；它不会随短租约释放或超时而自动消失。
所有 claim 路径统一使用事务级 advisory lock，避免“容量检查”和“lease 占用”之间出现竞态。
active 数量从 `agent_runs` 事实派生，不维护会因 crash 泄漏的 `active_count`：

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
-- 应用必须实际比较 active_count < $global_limit（MVP 默认 4）；
-- 不满足时 ROLLBACK，并记录 scheduler_capacity_unavailable。
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

Phase 0 必须提供可用的直接入口：

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
用户确认 Ready
```

最小 Requirement 表单：

```text
Project
Repository
Title
Description
Acceptance Criteria
Type
Priority
Related Requirements（Phase 0 仅信息关联；Phase 1 启用执行性依赖）
Model Policy（可选）
```

Requirement 不能在创建时直接启动 Agent，除非用户显式选择 `Start`。

状态语义固定为：`Ready` 表示可以被 Scheduler 自动领取。`POST /ready` 只确认需求进入
`Ready`；`POST /start` 对 Draft 执行同一 Ready 确认，对 Ready 幂等唤醒调度扫描，不绕过 lease、容量、
依赖或状态机检查。Phase 0 仍可把 `Start` 作为主要用户入口，但不因此关闭自动扫描。

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
  metadata: { created_at: string; created_by: string };
}

interface AcceptanceCriterion {
  id: string;          // AC-001 格式，平台自动生成
  description: string; // 必须可验证
  verification_method: "automated_test" | "ci_check" | "manual_review" | "gate_check";
  verification_ref: string; // 指向 ValidationStep、受信 Check selector 或人工验收规则
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
2. Agent 执行完成后，平台按 verification_method 逐项记录验证状态
3. MVP 对尚未具备 CI / Review 事实源的 AC 写入 `pending`；可在本地完成的 gate/test
   写入 `verified` 或 `failed`
4. 每次验证追加到 requirement_acceptance_checks，绑定验证批次和目标 commit，不覆盖旧证据
5. MVP：有效完成声明 + 封存产出 + 平台交接成功 → Submitted；AC pending 不等于 Gate 可跳过
6. Phase 1：PR 合并后建立最终验证批次，所有 AC 有合格证据或经授权豁免，且 CI/Review 满足 → Done
```

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
- Phase 1 跨 Repository 仅支持 `related_to`；跨仓交付物、部署版本等执行性依赖另行定义，
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
用户确认 Ready
    ↓
冻结 revision 和 digest
    ↓
Agent 执行
```

ADR/OpenSpec 是否存在不影响执行路径；Requirement Contract 始终是执行和验收的唯一输入。

### 7.3 Discussion

Discussion 是可选的需求澄清入口，但不是平台执行前提，也不是 Requirement 的状态来源。
Discussion 可以只保存对话，也可以帮助生成 Requirement Contract；平台不要求必须把讨论转成
ADR 或 OpenSpec。

```text
Discussion
├── conversation
└── optional requirement draft
```

典型流程：

```text
用户：增加登录功能
    ↓
Agent 澄清问题
    ↓
用户确认决策
    ↓
生成或完善 Requirement Contract
    ↓
用户确认 Ready
```

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

用户确认 Ready 时冻结输入；进入 Queued、开始 Run 或恢复交接时只验证已有冻结授权，
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

MVP 不运行独立 `DigestReconciler`，由 Scheduler / Supervisor 在以下时机同步计算：

```text
Ready 确认前
Agent start 前
提交产出 / 进入 Submitted 前
```

Phase 2 增加事件触发的 `DigestReconciler`，用于完整输入集合和跨 Requirement 影响分析。

#### 影响处理

```text
检测到变化
    ↓
比较旧/新 digest
    ↓
计算受影响 Requirements
    ↓
Ready / Queued / Running / Submitted / WaitingCI / WaitingReview
    → NeedsRevalidation
    ↓
停止新的 Agent dispatch
    ↓
重新生成当前 Agent Context
    ↓
用户或受信任的分析器确认：
  ├── no-impact → 记录授权与新快照；无有效产出时回 Ready，
  │              有有效封存产出时重新验证该产出并恢复交接/CI/Review 阶段
  └── affected → 修改 Contract/AC → 新 Revision → Ready；已合并时创建 follow-up，不复用旧 PR
```

MVP 状态机子集只涉及 `Ready` / `Running` / `Submitted`；`Queued` / `WaitingCI` /
`WaitingReview` 在 Phase 1 恢复完整状态机后适用。

no-impact/affected 的确认结果作为 `requirement_events` 持久化，记录操作者、理由、旧/新
snapshot、原 manifest、适用 revision 与授权 generation。Contract/AC 发生变化不能使用
no-impact 偷换 Revision；不变的 Contract 可以通过人工授权复用旧产出，但须重跑当前要求的验证。
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
AND 所有 AC-* 在该批次中 verified，或由允许的人工策略明确 waived
AND Run 输入快照完整，来源变化已分类且必要的重验证授权有效
AND 必要 Review 证据符合最终产出继承规则
AND 没有未解决的 `NeedsRevalidation`
```

“必要 Gate”按 Repository Policy 判断：本项目强制 Harness-Gate；外部仓库可使用已批准的
custom/none 策略。none 不是伪造 Gate pass，必须显式记录未配置门禁；Contract 指定的
gate_check 或 ci_check 仍需各自证据，不能随 none 自动通过。

默认采用合并后验证，不把 PR 合并事件当作验证通过：

- 自动测试和 gate_check：平台建立 `post_merge` VerificationBatch，在隔离环境检出最终 SHA，
  用当前受信策略执行，不启动编码 Agent。
- ci_check：匹配 policy 中固定的 Check 名称/发布 App 与精确 SHA，等待默认分支 CI 结果。
  缺少该 SHA 的结果保持 pending；超时进入 Blocked。平台不默认申请 Actions 重跑写权限。
- 合并前的 PR head、预合并 SHA 和最终合并 SHA 分别存储；不同 SHA 的 CI 不自动继承。
- manual_review：approval 必须来自允许 reviewer，未被撤销，明确关联 revision 和 AC。
  只有批准的源提交与最终提交的完整 Git tree 相同、且人工验收规则允许时，才可追加
  `review_tree_equivalence` 证据；保留原 approval 的 SHA，不将其改标为最终 SHA。
  不满足条件时，对最终 SHA 请求新的逐项人工验收。
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

`manual_review` 与未接入事实源的 `ci_check` 在 MVP 保持 pending。当前验收视图按
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
以及 Phase 1 的 SSE broadcaster。其他组件不直接向 Runtime 重复订阅，避免单消费者
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
sandbox 允许范围内的命令执行和文件修改 → 按冻结策略执行
create_local_commit → 受控工具按 Run/路径/分支/HEAD 校验后执行
可授权的越界请求 → 暂停并等待单次人工批准
未知工具、访问平台凭证或其他 workspace → 拒绝，不提供绕过硬边界的批准
人工输入请求（elicitation / user input）→ 暂停并等待人工处理
审批或人工输入超时 → Failed + manual_action_required = true
```

MVP 默认 Runtime sandbox：

```text
thread_sandbox = workspace-write
network = deny（只有 Repository Policy 显式白名单可开启）
```

审批策略必须绑定到 AgentRun 快照。普通配置更新仅影响新 Run；安全策略撤销会停止旧 Run，
不能以热更新方式偷偷放宽已有 session。

`workspace-write` 不等于 Git 元数据可写；默认 `.git` 及 worktree 的关联 Git 目录受保护。
MVP 不通过关闭沙箱来解决本地提交，而是注册受控工具：

```text
create_local_commit(paths, message, expected_head)
```

LocalGitBroker 由平台维护，以当前 Run 的 generation/dispatch_epoch 校验授权，只接受当前
workspace 的明确相对路径；验证真实 worktree、目标 branch、expected_head 与 diff 范围。
使用固定 Git 二进制、参数数组和受信配置，禁用仓库自带 hooks、credential helpers 及未批准
filters；Gate hook 由独立隔离验证器执行。Broker 不持有 GitHub 凭证，不提供任意命令、rebase、
force push 或其他仓库写入接口。Agent 可请求提交，但不能直接修改共享 refs 或 canonical clone。

审批/人工输入请求写入 `runtime_requests`，保存平台 UUID 与原始协议 request ID、Run/thread/turn、
generation、请求类型、内容摘要、过期时间及状态。回答先做 CAS 决策落库，再回复准确的原请求；
重复相同回答幂等，冲突回答、超时回答、取消后回答及旧 incarnation 的回答拒绝。
API 和 UI 从 Phase 0 起支持该闭环；尚无 UI 时也必须可通过鉴权 API 回答。

Agent 运行完成必须有显式完成信号：

```text
report_completion(summary)
```

平台接受该工具时读取受控工作区 HEAD，并将声明与 Run/revision/digest/generation 原子持久化；
数据库提交成功后才回复工具成功。随后禁止新的写入工具调用，中断 session，确认整个执行组
退出，再验证 HEAD 未偏离声明、工作区无未提交改动、相对 change_base 有真实产出以及 Gate 通过。
对复用的中间 commit 必须校验来源并重新验证；不能仅凭历史 Gate 结果认定完成。
验证器在不可变 SHA 的独立检出上工作，受信 Gate 和封存完成后 AgentRun 才 Succeeded。
声明、产出清单和交接记录的绑定规则见 3.5，之后不再恢复这个 session 继续修改。
如果 turn 结束没有完成信号且工作区没有新变更，连续两次视为
`agent_completion_not_reported`；如果有新变更则允许继续下一 turn，直到 `max_turns`。

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
处理事件
    ├── notification
    ├── tool call
    ├── approval
    ├── token usage
    └── turn completed
    ↓
未声明完成且授权仍有效 → 同一 thread 继续下一 turn（受 max_turns 限制）
    ↓
report_completion → 持久化声明 → 禁止新写入 → 停止整个执行组并确认退出
    ↓
平台验证声明 HEAD / 输入资格 / 受信 Gate → 封存 manifest
    ↓
AgentRun = Succeeded → 释放执行租约 → HandoffOperation
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
- `create_local_commit` / `report_completion` 使用 `dynamicTools`。按目标版本开启所需
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
7. 真实 worktree 中沙箱直接写 .git 被拒绝，create_local_commit 可完成受控提交
8. dynamicTools 注册/调用/回包有效；完成声明持久化后重启不会丢失或重复交接
```

协议和安全参考（适配时以锁定版本复核）：

```text
https://developers.openai.com/codex/app-server/
https://learn.chatgpt.com/codex/agent-approvals-security
```

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
```

Context 必须有：

```text
context_version
source_refs
generated_at
redaction_status
token_estimate
```

不要把全部 CI 日志、全部历史事件和全部 Repository 内容直接塞给 Agent。

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
- 全局并发限制（Phase 0 / Phase 1）
- 项目并发限制（Phase 3）
- Repository 并发限制（Phase 3）
- retry 上限
- Prompt 版本
- hooks
- 模型策略

MVP 仅启用以下动态更新：

```text
全局并发限制、retry 上限、Prompt 版本、hooks 和模型策略
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

成功执行后的 workspace 是否立即删除由 Repository policy 决定；默认保留一段时间，便于排障。

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
10. push 非预期拒绝时先 fetch 对账；禁止自动 force push。需要 rebase 时显式开始新执行，
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

GitHub App 权限按最小集合申请：

```text
contents: write
pull_requests: write
checks: read
actions: read
metadata: read
issues: write（仅当启用 Issue 集成时）
commit_statuses: write（仅当需要设置 commit status 时）
deployments: write（Phase 5 自动部署时）
```

Webhook 不是普通 repository permission，而是 GitHub App 的事件订阅配置。Phase 1 至少订阅：

```text
push
pull_request
pull_request_review
pull_request_review_comment
check_run
workflow_run
check_suite
```

Phase 1 安装范围为用户个人仓库；组织仓库权限留到后续版本。

### 11.2 Webhook 路由

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

---

## 12. CI 闭环

### 12.1 Phase 1 CI 目标

Phase 1 先读取 GitHub Checks / Actions 的结构化结果：

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

```text
PR Open
    ↓
CI Queued
    ↓
CI Running
    ├── Passed → WaitingReview
    └── Failed → Failure Analysis
```

### 12.3 Failure Summary

发送给 Agent 的不是完整原始日志，而是经过限制和脱敏的摘要：

```text
CiFailureSummary
├── repository
├── pull_request
├── commit_sha
├── workflow
├── job
├── step
├── conclusion
├── error_excerpt
├── annotations
├── relevant_files
├── log_artifact_refs
└── possible_causes
```

需要：

- 最大字节数
- Secret 脱敏
- 日志来源
- commit 对齐
- 失败发生时间
- 是否允许自动修复

### 12.4 Recovery Requirement

CI Failure 生成 Recovery Requirement：

```text
CiRecovery Requirement
├── root_requirement_id
├── source_ci_failure_id
├── target_pull_request_id
├── target_branch
├── parent_id
├── total_recovery_attempts
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

### 12.5 修复次数上限

本节只在 Phase 2 启用；MVP 不创建 Recovery / ReviewFix。

每个 root Requirement 的总修复次数硬上限为 3：

```text
total_recovery_attempts = CiRecovery 次数 + ReviewFix 次数
                        <= 3
```

ReviewFix 必须等 CI 通过后才运行：先修 CI，再修 Review，避免同时处理两类问题。

超过上限：

```text
root Requirement → Blocked
不自动重试
需要人工审查、修改 Contract 或显式 Unblock
```

数据模型预留：

```sql
ALTER TABLE requirements ADD COLUMN root_requirement_id UUID;
ALTER TABLE requirements ADD COLUMN total_recovery_attempts INT DEFAULT 0;
```

以 root 级 `total_recovery_attempts` 为唯一计数口径。创建新修复 Requirement 时在 root 行锁内
检查并递增；重复事件、同一子任务的基础设施重试与交接重试不重复计数。Unblock 不清零总次数，
达到上限后只能人工完成当前修复或创建新的显式需求，不能用 Unblock 无限增加自动修复任务。

### 12.6 本项目 Harness-Gate 门禁集成

Personal AI Software Factory 自身仓库使用 Harness-Gate 作为确定性工程质量门禁。它不替代
Codex harness，也不替代 GitHub Actions；它负责在本项目的本地提交、Agent 自检、PR、CI
和发布流程中统一执行可复现的质量检查。

参考实现：

```text
https://github.com/musutrade/Harness-Gate
```

初始接入基线固定到经过验证的不可变版本，例如 `v0.3.7`。本项目不得使用 `latest`、`main`
或可变下载 URL；升级 Harness-Gate 必须显式修改版本并重新跑完整门禁。

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
harness_gate_binary
harness_gate_version
harness_gate_config_path
harness_gate_profile
harness_gate_config_digest
trusted_gate_policy_revision_id
trusted_source_commit_sha
protected_entrypoint_digests
evidence_store_ref
```

Repository 使用通用的 `gate_provider`、`gate_config`、`gate_recovery_policy` 和受信 policy 指针；
`harness_gate_*` 字段只作为 AgentRun / Gate invocation 的不可变审计快照，不与
Repository 配置重复双写。

仓库内的配置是策略源，不是“Agent 当前工作区有什么就执行什么”。管理员从固定源 commit
批准 policy revision，固定配置、强制步骤集合、工具版本和受保护验证入口的摘要。
Agent 修改这些路径时，变更保持待批准；不能自动把工作树的新摘要当作受信摘要。
批准需要鉴权 API、操作者和理由，Agent 工具没有策略批准权限。

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

`doctor` 可以按本项目配置摘要和 Worker 主机缓存，但不能跳过首次环境验证。Agent 不能
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

失败结果进入 AgentRun：

```text
gate_status = failed
gate_failure_code
gate_retry_class
gate_report_path
gate_invocation_id
```

本项目门禁失败不应直接生成新的普通 Feature。MVP / Phase 0 不生成 `CiRecovery`；
Phase 2 只有满足 `gate_recovery_policy` 且错误属于 fixable 白名单时，才生成关联原 PR
的 `CiRecovery` Requirement。

#### 门禁失败后的处理策略

默认 fail-closed 表示失败时禁止提交/封存/交接，不等于所有失败都禁止有限修复。
处理优先级是安全硬规则、执行阶段、Repository Policy，最后才是错误码默认重试：

```text
所有阶段：
  config / secret / architecture / evidence / policy 撤销等失败立即停止，
  Phase 0 → Failed + manual_action_required；Phase 1+ → Blocked

完成声明前的受信 hook：
  仅 test / lint / build 失败可作为 create_local_commit 的失败结果反馈给当前 Agent；
  不提交，允许当前 Run 在 max_turns / wall-clock 上限内修代码后再次请求；
  不创建 Recovery，不消耗外部交接重试计数，不能自行降级门禁

声明后的平台验证（Phase 0 / Phase 1）：
  test / lint / build 失败 → Failed（Phase 0）/ Blocked（Phase 1）并人工处理；
  不重开已结束的 session，不生成 CiRecovery；
  纯验证基础设施超时仅重试同一固定 SHA 的验证步骤，次数见第 28 章

Phase 2 外部仓库：
  由 Repository Policy 的 gate_recovery_policy 显式配置，默认 blocked
  显式启用后，仅 fixable 类失败（test_failed / lint_failed / build_failed）
  可自动生成 CiRecovery
  config_invalid / secret_detected / architecture_violation 永远 blocked
  不因任何配置自动修复
```

Phase 2 的 CiRecovery 只针对已有 PR；本项目自身仓库仍不自动生成门禁修复任务。
阶段内 hook 反馈与新的 Recovery Requirement 是两条不同路径，均不能覆盖失败事实。

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

---

## 13. PR Review 闭环

### 13.1 Review 事件

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

只有满足以下条件才进入自动修复候选：

- 评论来自允许的 reviewer
- 评论未解决
- 评论不是纯通知
- 评论没有被标记为 ignored
- PR 仍然开放
- 没有其他 Recovery / ReviewFix 正在修改同一 branch

ReviewFix 记录：

```text
source_review_comment_id
target_pull_request_id
target_branch
root_requirement_id
parent_id
review_text
file_path
line
```

### 13.3 ReviewFix 执行

```text
PR Review Comment
    → Review Analysis
    → ReviewFix Requirement
    → 原 PR branch lease
    → Agent 修改
    → Commit / Push
    → CI
    → 重新 Review
```

ReviewFix 与 CiRecovery 共享 root 级 `total_recovery_attempts` 计数（上限 3，见 12.5），
超限后 root Requirement 进入 `Blocked`，需要人工 Unblock。ReviewFix 必须等 CI 通过后才能运行。

---

## 14. Persistence 设计

### 14.1 PostgreSQL

在多 Project、多 Repository、多 Agent 和重启恢复场景下，PostgreSQL 是合理的 Phase 1 选择。

Redis 和 MQ 不作为 Phase 1 必需依赖：

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
completion_declarations
artifact_manifests
runtime_requests
handoff_operations

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

model_providers
models
model_policies

settings
```

`requirements` 的执行授权与当前视图字段：

```text
current_revision_id          UUID，指向唯一可执行 Contract Revision
state_version                BIGINT，领域命令 CAS 版本
generation                   BIGINT，当前执行/交接授权代次
frozen_context_input_digest  TEXT，Ready 时冻结的当前输入摘要
root_requirement_id          UUID，根 Requirement 的值为自身 ID，子任务指向根
total_recovery_attempts      INT DEFAULT 0，仅根 Requirement 维护，CiRecovery + ReviewFix 合计
gate_recovery_policy         JSONB NOT NULL DEFAULT '{"mode":"blocked","allowed_failure_codes":[]}'
failure_code                 TEXT NULL
retry_class                  TEXT NULL
manual_action_required       BOOLEAN NOT NULL DEFAULT false
```

`requirements` 不保存 Contract JSON 副本、产出提交字段或第二份 AC 副本；
Contract 及其 schema 校验只存在 `requirement_revisions.contract` /
`requirement_revisions.contract_version`，产出提交只以 `agent_runs.artifact_commit_sha`
为准。

### 14.3 关键约束

```text
requirements(project_id, repository_id, ...)
requirement_revisions(requirement_id, revision) UNIQUE
requirement_acceptance_checks(verification_batch_id, criterion_id, producer_event_id) UNIQUE
completion_declarations(agent_run_id) UNIQUE
artifact_manifests(agent_run_id) UNIQUE
runtime_requests(agent_run_id, worker_incarnation_id, protocol_request_id) UNIQUE
outbox_events(handoff_operation_id, action_key) UNIQUE
webhook_events(provider, delivery_id) UNIQUE
worktree_leases(repository_id, branch) UNIQUE
pull_requests(provider, repository_id, number) UNIQUE
CHECK (recovery_source_type IS NULL OR recovery_source_type IN ('ci_failure', 'review_comment'))
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

`total_recovery_attempts <= 3` 只在 Phase 2 启用 Recovery / ReviewFix 时作为领域命令的
事务性检查，不作为 MVP 的全局表约束。

另需约束：

- 当前 Revision、snapshot、声明、manifest、HandoffOperation 必须属于同一 Requirement；
  使用组合外键或等价事务校验，不能只验证 UUID 存在。
- 同一 Repository/branch 同时只能有一个未终结交接；同一 Run 的同一 generation 不重复创建
  HandoffOperation。跨 generation 复用产出必须引用人工重验证授权。
- gate_policy_revisions、声明、manifest、验证证据追加后不可更新；策略撤销以单独事件记录。
- 验收记录的 producer_event_id 是稳定的幂等键；新状态通过新事件追加，当前验收视图按目标
  SHA/批次派生，不保留 `(revision, criterion)` 全局唯一约束。

### 14.4 事务边界

核心事务：

| 命令 | 同一事务中的写入 |
|---|---|
| Ready/Start | 校验状态版本与 Contract、冻结 snapshot、授权 generation、领域事件 |
| 接受完成声明 | 校验当前租约/generation、插入声明、Run → Finishing、领域事件；提交后回复工具 |
| 验证和封存完成 | 引用已可靠保存的 sealed_ref/证据、插入 manifest、活跃 Run → Succeeded、创建交接和 outbox；人工重验证不改写终态 Run |
| Cancel/输入替代 | 递增 generation/state_version、撤销待处理 runtime_requests、标记未发出命令失效、领域事件 |
| 交接成功 | 校验当前授权和远端事实、持久化 PR 关联、交接成功、Requirement → Submitted |
| 创建 Recovery | 锁 root 并检查总上限、来源去重、创建子任务及关联、递增计数、事件/outbox |
| Done | 绑定最终验证批次、输入资格及全部证据、写不可变完成快照和领域事件 |

Git 对象和证据先写入平台私有的不可变存储并确认可读取，再提交引用它们的数据库事务；重启后的
孤立文件可清理，数据库不能先声明一个尚未保存的 manifest 已封存成功。
外部 GitHub API 调用不能放在长事务中：

```text
封存产出 → 交接/outbox → 领取一个有授权的动作 → 外部调用 → 事实归档 → 状态推进
```

每个 outbox 动作包含稳定 action_key、manifest_id、generation、repository/branch、目标 SHA、
预期远端 SHA、调用状态及尝试次数。Handoff Worker 的执行规则：

1. 在短事务内取得操作与 Repository lease，校验当前 generation、可交接状态、branch reservation、
   输入资格和有效验证批次的 Gate policy 未撤销，将动作标记 `Sending`；这是本次调用的授权时点。
2. push 只发送 manifest 固定 SHA，必须使用预期远端 old SHA 的条件更新且只允许 fast-forward。
   远端已是目标 SHA 则幂等成功；不符时先对账，绝不自动 rebase 或无条件 force push。
3. PR 创建先按 repository + 精确 head branch + base branch 查找，并校验远端 head。
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

### 15.1 Phase 1

Phase 1 只需要支持：

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

但 Phase 1 先记录 Token 和成本，不强制自动封锁，除非预算是产品的核心卖点。

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

`ready` 对 Draft 确认并冻结输入，对相同版本的 Ready 幂等；`start` 对 Draft 先执行同一确认，
对 Ready 直接唤醒一次扫描，其他状态拒绝。两者不能绕过依赖、容量、lease 或输入资格检查。

所有写命令携带 `Idempotency-Key` 和期望 `state_version`，通过领域事务校验，冲突返回 409。
Retry/Unblock 默认按 `failure_phase` 恢复：handoff 继续交接，validation 重新验证固定 SHA，
execution 才回 Ready 创建 Run。显式重新编码必须先撤销旧交接并完成对账。
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

POST   /webhooks/github
```

Webhook endpoint 不提供通用写入 API，所有外部写入必须经过签名验证、去重和事件解析。

### 16.6 SSE

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

上述路径规则不替代操作系统隔离。从 Phase 0 起，Agent 工具、after_create/before_run/after_run、
依赖安装脚本以及 Gate/test 子进程都在低权限执行环境运行，不继承 API/调度器的执行身份。
必须限制可读挂载和可写目录、网络出口、进程/CPU/内存额度；禁止挂载 Docker socket、
平台数据库凭证、GitHub App 私钥、API Bearer Token 和 evidence store 的写权限。

受信控制进程负责判定和归档，仓库代码只提供不可信输出。执行组以 cgroup、容器或等价可验证
的 OS 边界标识并完整终止，不能仅按 PID 杀单进程。生产就绪检查必须实际验证凭证不可读、
其他 workspace 不可访问、子进程不可逃逸；环境不具备这些能力时拒绝启动写入型 Run。

### 17.3 凭证

凭证只允许存在于：

```text
GitHub App credential provider
Model secret provider
Runtime host environment
```

不得：

- 写入 Requirement
- 写入 Agent Context
- 写入日志
- 写入 WORKFLOW.md
- 通过普通环境变量直接继承到 Agent

Codex app-server 的模型认证与工具子进程的可见环境分开：只读挂载认证文件本身并不能阻止
读取，必须验证工具执行视图无法访问该文件及父进程凭证；若锁定版本/宿主隔离不能满足，
使用受控认证代理，或拒绝该部署模式。GitHub 凭证只由交接控制端持有，LocalGitBroker、
hooks 和验证步骤均不持有。日志脱敏不是凭证隔离的替代品。

### 17.4 API 和 Webhook

必须：

- MVP 使用静态 Bearer Token：请求必须携带
  `Authorization: Bearer <token>`；
- Token 只从 Runtime host 的 secret provider 读取，不写入仓库、`WORKFLOW.md`、数据库明文
  或日志；
- 生产环境通过 secret provider 轮换 Token，并拒绝缺少或格式错误的 Authorization；
- CSRF 防护
- GitHub webhook HMAC 验证
- webhook replay 防护
- 请求体大小限制
- API rate limit
- 敏感字段脱敏

Phase 0 只开放 API；Webhook 的签名、重放和 inbox 处理随 Phase 1 开启。生产 API 使用
TLS、受限监听地址和明确的 CORS allowlist；Agent 执行环境不能访问该管理入口。

### 17.5 自动 Merge

Phase 1 默认不自动 Merge。

如果启用，必须同时满足：

```text
CI passed
AND
branch protection passed
AND
no unresolved review comments
AND
required reviewer approval
AND
Requirement acceptance criteria passed
AND
no active Recovery / ReviewFix
```

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

首页采用 `My Development / Work Queue`，显示：

```text
Running Agents
Ready Requirements
Submitted Requirements
Recent Activity
```

Phase 1 起再增加：

```text
Blocked Items
PRs Waiting Review
CI Failures
```

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

Phase 1 增加 queued_requirements、blocked_requirements、ci_failure_count；
Phase 2 启用 ReviewFix 后再增加 review_fix_count。

V2：

```text
OpenTelemetry
Prometheus
Grafana
```

### 19.4 Dashboard

首页不是传统 Admin Dashboard，而是工作队列：

```text
My Development
├── Running Agents
├── Ready Requirements
├── Submitted Requirements
└── Recent Activity

Phase 1：
├── Queued Requirements
├── Blocked Items
├── PRs Waiting Review
└── CI Failures
```

MVP 不展示尚未启用的 CI / Review / Blocked 统计；这些卡片在 Phase 1 启用对应事实源后
才显示。

必须支持按以下维度过滤：

```text
Project
Repository
Requirement State
Agent State
PR State
CI State
```

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
- global capacity（Phase 0 / Phase 1 默认 4）
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

---

## 21. 部署

### 21.1 Phase 0 / Phase 1 Docker Compose

```text
docker compose
├── app                  # API/调度/可信控制面
├── executor             # 低权限本机执行服务，无平台数据库与 GitHub 凭证
└── postgres
```

executor 是同一主机的安全边界，不是多 Worker 调度或新增消息队列。控制请求使用本机受限
通道和按 Run 限定的能力；不提供任意平台命令执行。每个 Run/验证任务使用独立执行组。

Workspace 使用显式挂载：

```text
./workspaces:/workspaces
```

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
SSE broadcaster
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

---

## 22. 测试和验证矩阵

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
- no-impact 确认后生成新冻结 snapshot
- 失效且未经重验证授权的 snapshot 不能用于 Done
- Phase 0 拒绝执行性依赖；Phase 1 校验上游 Done 与基线祖先关系

### 22.2 Scheduler

- Phase 0：单 Project / 单 Repository 下验证全局容量上限不会突破（同仓串行时有效运行数
  通常为 1）
- Phase 1：多 Project 并行
- Phase 1：多 Repository 并行
- Phase 0：全局容量限制的上限不超限；Phase 1 再验证并行吞吐
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
- Phase 2：同一 PR branch 的 Recovery / ReviewFix 互斥

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
- Phase 0：受控 Git 提交、并行审批和 user input 的重复/超时/迟到回答

### 22.5 GitHub

- 多 installation
- 多 repository webhook 路由
- signature 校验
- delivery 去重
- webhook 乱序
- PR 关联
- CI 结果归档
- Phase 1：Review 事实归档
- Phase 2：ReviewFix 生成

### 22.6 Recovery

- Phase 2：CI failure 创建 Recovery Requirement
- Phase 2：复用原 PR branch
- Phase 2：最大修复次数
- Phase 2：重复事件不重复创建任务
- Phase 2：修复失败进入 Blocked
- Phase 1：人工 Unblock 按受阻阶段恢复
- Phase 2：达到 root 总上限后 Unblock 不重置计数

### 22.7 输入一致性

- Requirement Contract 修改影响分析
- MVP：knowledge_policy 实际读取文件 `path + blob_oid` 变化检测
- Phase 2：Repository policy / workflow / Harness-Gate 完整 digest 变化检测
- 冻结 digest、来源分类和人工重验证授权联合校验
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

### 22.9 真实集成测试

Phase 0 真实集成至少验证：

```text
创建 Requirement
→ Worktree
→ Agent 修改 / create_local_commit
→ report_completion 持久化 / 执行组静止 / 受信验证 / 封存
→ HandoffOperation 幂等 push / 创建 PR
→ Submitted
```

在声明前、声明后、封存后、push 后和 PR 创建后分别注入重启；验证各自恢复路径，
而不是要求所有重启都创建新 AgentRun。

Phase 1 真实集成至少需要一个 disposable GitHub Repository，验证：

```text
创建 Requirement
→ Agent 修改
→ Worktree
→ Commit
→ Push
→ PR
→ CI
→ Review / 手动 Merge
→ 最终 SHA 验证批次 / 人工证据继承检查
→ Reconciler 进入 Done
```

真实测试失败不能静默当作通过。

### 22.10 故障窗口验收矩阵

| 阶段 | 注入场景 | 必须观察到的结果 |
|---|---|---|
| Phase 0 | 只有中间 commit，未 report_completion 就崩溃 | 不 push、不补建 PR；终止旧组后按预算新建 Run |
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
| Phase 1 | PR 修改所选知识文件后合并 | 正确分类自有产出并继续最终验证 |
| Phase 1 | 同路径存在外部输入变化 | 进入 NeedsRevalidation，不自动豁免 |
| Phase 1 | PR CI 成功但最终 SHA 无证据 | 保持 pending，不能 Done |
| Phase 1 | 同一 AC 在两个 SHA 上先成功后失败 | 保留两份历史，最终视图使用正确目标 SHA |
| Phase 1 | 上游仅 Submitted 或不在下游基线 | 不领取下游，不消耗 retry |

---

## 23. 实施分期

### Phase 0：执行内核

目标：先证明单 Project、单 Repository 的端到端闭环可行；终点是 PR 创建并进入
`Submitted`，不是 `Done`。

包含：

- Project / Repository 基础表
- 多 Project 数据模型，Phase 0 使用限制为单 Project
- Requirement 结构化 Contract + 自动 AC ID + 不可变 Revision + AC 验收证据
- MVP 状态机：Draft / Ready / Running / Submitted / Failed（+ Cancelled、NeedsRevalidation）
- `context_input_digest`（Contract + AC + WORKFLOW.md + 实际读取知识文件的
  `path + blob_oid`；本地未提交例外为 content hash）
  与 `NeedsRevalidation`
- 一个 CodexRuntime：直接实现，参考 Symphony app_server.ex，schema codegen，版本锁定
- 调度：全局并发 4 + 同 Repository 串行 + FIFO（priority 排序）+ claim lease
- Workspace / Worktree 创建和清理
- AgentRun
- 基础日志
- Reconciler：Process + Workspace；按声明/manifest/交接阶段恢复，先处理旧执行组
- 受控 LocalGitBroker、dynamicTools、审批/user input API 和基础 UI
- 完成声明、产出清单、Handoff Worker/outbox 及独立交接重试
- generation/dispatch_epoch、恢复屏障和取消竞态处理
- 低权限 executor、受信 Gate policy 批准/撤销与私有证据存储
- Scheduler 自动扫描并领取 `Ready`；保留手动 `Start` 作为立即触发一次扫描的用户入口
- 本项目 Harness-Gate 接入（config check、doctor、hook、封存前 verify --profile ci --all）
- 本项目 Angular 设计 Token、页面骨架和基础可访问性基线
- 错误码表与 MVP RetryPolicy 引擎（第 28 章）

不包含（推迟清单见本章末尾）：

- 多 Repository 调度、Project 级容量、公平调度
- WaitingCI / WaitingReview / Merging / Done / Blocked 状态
- CI Recovery、ReviewFix
- 阻塞调度的执行性依赖（仅保留 related_to 信息关联）
- 完整 policy / Harness-Gate digest
- SSE 实时更新
- Recovery 重试、ReviewFix 重试和 root 级修复计数

验收：

```text
创建一个 Requirement，确认 Ready（或调用 Start 立即触发扫描），
Agent 在独立 worktree 中修改代码并请求受控本地提交；
声明完成后平台停止执行组、验证封存并幂等交接 PR；
UI 区分执行/审批/验证/交接，重启后不误交接中间 commit、不重复编码已封存产出。
```

### Phase 1：CI / Review / Webhook 主闭环

包含：

- 多 Repository 映射（恢复多仓库能力）
- Checks / Actions 读取
- Webhook inbox
- Reconciler：增加 GitHubReconciler
- `InputSnapshotChanged` 事件与受影响 Requirement 重验证
- 增加 WaitingCI / WaitingReview / Done / Blocked，以及人工 Unblock
- `Submitted → WaitingCI → WaitingReview → Done`
- Phase 1 默认人工 Merge；合并事件创建最终验证批次，条件全部满足后才 Done
- 最终 SHA 的 CI/Gate 证据、逐项人工验收和可审计的 Review 继承规则
- 输入来源分类、OutputIntegrated 与 no-impact 产出重新授权
- 同 Repository 执行性依赖，校验上游 Done 和下游基线包含关系
- 基础 Angular 工作队列
- SSE 实时更新

验收：

```text
多个 Repository 的 Requirement 可以并行走到 PR，
服务重启后不会重复运行同一个 branch，并能基于外部事实推进到 WaitingReview；
人工 Merge 后对精确的最终 SHA 验证，通过后才由 reconciler 推进到 Done。
```

### Phase 2：CI Recovery 和 ReviewFix

包含：

- Failure Summary
- Recovery Requirement
- Review Comment 归档
- ReviewFix Requirement
- 原 PR branch 复用
- root 级 `total_recovery_attempts` 硬上限 3 + 同 branch 串行
- Blocked 人工处理
- 完整输入 digest（Project/Repository policy、Harness-Gate 配置和完整上下文）
- Reconciler：增加 DigestReconciler 和 ResourceCleaner

### Phase 3：可选需求澄清增强

包含：

- Discussion
- Discussion messages
- 从 Discussion 生成 Requirement Contract 草稿
- Requirement 自动拆分
- 仓库文档输入的 UI 预览和引用展示
- 多 Project 公平调度与 Project 级容量

ADR/OpenSpec 的文件读取由 Project/Repository 的 `knowledge_policy` 控制，不需要独立 Phase。
不采用 Discussion 时可以跳过澄清模块，公平调度独立启用；Phase 0 的 Contract、Revision、
context_input_digest、漂移检测和验收证据仍然是强制能力。

### Phase 4：模型和体验增强

包含：

- Model Policy
- Fallback
- 预算
- 多 Runtime
- 移动 PWA
- 通知
- 高级 CI 分析

### Phase 5：分布式扩展

包含：

- Remote Worker
- 多机器调度
- Redis Streams / NATS
- 跨 Repository Requirement
- 自动部署

#### 推迟清单

| 能力 | 推迟到 | 回归条件 |
|---|---|---|
| WaitingCI / WaitingReview / Done 状态 | Phase 1 | 恢复 CI/PR 闭环；`Merging` 仅未来自动 Merge |
| Project 级容量、公平调度 | Phase 3 | 多项目并行有饥饿现象 |
| 完整输入 digest（policy / Harness-Gate / 完整上下文） | Phase 2 | MVP blob_oid digest 稳定 |
| 事件溯源重放 | Phase 3 | 审计/时间旅行成为真实需求 |
| 数据保留任务、指标看板 | Phase 1 后 | MVP 上线稳定 |

#### 里程碑顺序

不设单点工期估算，只定里程碑顺序：

```text
1. CodexRuntime 直接实现（验收标准见 8.3）
2. 端到端闭环：Requirement → Agent → PR → Submitted（MVP）
3. 多 Repository + CI/PR 状态同步（Phase 1）
4. Recovery / ReviewFix（Phase 2）
```

---

## 24. MVP Definition of Done

MVP 必须满足：

### 领域

- [ ] 多 Project 数据模型（MVP 使用限制为单 Project）
- [ ] 多 Repository 数据模型（MVP 使用限制为单 Repository）
- [ ] Requirement 一对一绑定 Repository
- [ ] MVP 状态机：Draft / Ready / Running / Submitted / Failed + Cancelled + NeedsRevalidation
- [ ] related_to 信息关联；明确拒绝 MVP 的 blocks/depends_on
- [ ] AgentRun 历史
- [ ] 结构化 Requirement Contract + 平台自动生成 AC ID
- [ ] Requirement Revision 不可变
- [ ] `context_input_digest`（Contract + AC + WORKFLOW.md + 实际读取知识文件的
      `path + blob_oid`；本地未提交例外为 content hash）
- [ ] `NeedsRevalidation` 漂移状态
- [ ] 最小 `InputSnapshotChanged` 检测与处理
- [ ] `knowledge_policy` 透传读取（不解析）+ 20 文件 / 100KB / 1MB 限制
- [ ] 按批次/目标 SHA 追加 AC 证据，MVP 未接入验证源时 pending
- [ ] 声明、manifest、generation、输入资格和 Gate 全部有效才允许 Submitted
- [ ] 本项目 `.harness-gate/` 配置和固定版本

### 调度

- [ ] 全局并发限制（默认 4）
- [ ] 同 Repository 串行
- [ ] FIFO + priority 排序
- [ ] claim lease（60s 超时 / 20s 心跳）
- [ ] MVP 临时故障 retry / backoff（startup / response / stall / git fetch / model 429/5xx）
- [ ] 按故障窗口恢复：无声明不交接、有 manifest 不重新编码
- [ ] lease 看门狗、旧执行组退出确认、dispatch_epoch 和 branch reservation

### 执行

- [ ] CodexRuntime 直接实现，8.3 的全部验收通过
- [ ] 独立 workspace
- [ ] Git Worktree
- [ ] 分支保护
- [ ] timeout / stall
- [ ] after_create / before_run / after_run hooks
- [ ] create_local_commit 和 report_completion 的真实动态工具闭环
- [ ] 审批/user input API 与 UI，覆盖并行、重复、超时及取消后回答
- [ ] 本项目 Agent 自检必须通过 Harness-Gate
- [ ] 错误码表 + RetryPolicy 引擎（第 28 章）

### GitHub

- [ ] GitHub App 最小安装与 PR 创建能力
- [ ] PR 创建
- [ ] PR 与 Requirement 关联
- [ ] HandoffOperation 独立重试、Sending 授权与取消/输入替代对账
- [ ] push/PR 响应丢失不重复 PR，也不创建新的 AgentRun

### 观测

- [ ] Agent Event
- [ ] Requirement Event
- [ ] 结构化日志
- [ ] 基础 Dashboard
- [ ] 门禁 invocation、failure code 和报告可追踪

### 本项目 UI

- [ ] 语义 Token 分层
- [ ] 标准页面骨架
- [ ] Loading/Success/Empty/Failure 状态
- [ ] Desktop Chrome 与 Pixel 7 视觉验收
- [ ] 键盘、焦点、暗色和 reduced-motion 验收

### 安全

- [ ] API 鉴权
- [ ] Secret 不进入 Agent
- [ ] hooks/安装脚本/Gate/test 的隔离和凭证不可读验证
- [ ] 受信 Gate policy 独立批准/撤销，Agent 不能修改权威证据
- [ ] workspace containment
- [ ] 自动 Merge 默认关闭

### 推迟项（不进 MVP，保留回归路径）

- [ ] WaitingCI / WaitingReview / Done 状态（Phase 1；`Merging` 仅未来自动 Merge）
- [ ] 多 Repository 调度、Checks 读取（Phase 1）
- [ ] Webhook 接收、签名验证、去重和事件处理（Phase 1）
- [ ] webhook replay 防护（Phase 1）
- [ ] SSE 实时更新（Phase 1）
- [ ] Project 级容量、公平调度（Phase 3）
- [ ] CI Recovery / ReviewFix（Phase 2）
- [ ] 完整输入 digest（policy / Harness-Gate / 完整上下文，Phase 2）
- [ ] 数据保留任务、指标看板（Phase 1 后；指标先采集不设阈值）

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
| 管理 API、权限与部署隔离 | 16～17、21 |
| 故障窗口、阶段范围与 MVP 验收 | 22～24 |
| 错误分类、阶段动作与重试预算 | 28 |

---

## 26. 主要风险

### 风险 1：Agent 自动修改范围过大

缓解：

- 独立 workspace
- 独立 branch
- GitHub branch protection
- 执行前后 diff 检查
- 最小权限
- 默认关闭自动 Merge

### 风险 2：Webhook 重复或乱序

缓解：

- delivery_id 唯一约束
- inbox/outbox
- 状态版本号
- Reconciler 定期对账

### 风险 3：修复循环消耗大量模型额度

缓解：

- root 级 `total_recovery_attempts <= 3` 硬上限
- 执行/验证/交接分别限次，交接失败不再调用模型
- Phase 4 增加预算限制；前期先采集成本
- Blocked 状态 + 人工 Unblock

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

> 一个面向个人开发者的、多项目多仓库、以 Requirement 为核心、以 Agent 为执行单元、以 GitHub 为协作基础设施的 AI 软件研发自动化平台。

它继承 Symphony 的执行内核，但将单项目参考实现升级为：

```text
多项目配置（数据模型）
多仓库调度（Phase 1 起）
全局公平容量管理（Phase 3 起）
持久化状态
Webhook 对账
可恢复 Agent Run
```

交付路径以 MVP 最小闭环为起点（第 23 章），上述能力按 Phase 渐进恢复，不在 V1 一次性铺开。

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

Requirement 覆盖不能把 `blocked` 改成自动重试，也不能超过阶段硬上限。

错误记录必须带 failure_phase，不能仅凭 retry_class 决定是否启动新的 AgentRun：

| failure_phase | 自动恢复动作 | 等待状态与计数 |
|---|---|---|
| execution | 停止旧组并确认静止，无有效声明/manifest 时新建 Run | Ready + not_before_at；execution_attempt |
| validation | 重试固定 SHA 的受信验证，不重启模型 | 封存前 Run 保持 Finishing；verification_attempt |
| handoff | 对账并重试同一 push/PR 步骤 | Running + Handoff RetryWaiting；operation_attempt |
| post_merge | 等待精确 SHA 的 CI 或重试平台验证 | WaitingCI/WaitingReview；verification_attempt |

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
远端 head 冲突不进入临时重试；git_push_rejected 不自动 rebase。

MVP 不创建 CI Recovery/ReviewFix。完成声明前 hook 的 test/lint/build 失败按 12.6.4
反馈给当前 Agent，不能视为新的 Run 重试；声明后的 fixable 失败转人工。

MVP 的 RetryPolicy 只允许以下自动重试：

```text
transient / recoverable 且属于当前阶段 allowlist → 在同一阶段有限重试
声明后的 fixable / blocked / 未知错误            → Failed + manual_action_required=true
```

MVP 每种阶段的自动重试总预算最多 2 次，单错误上限取 min(表中默认值, 2)；执行预算按
Requirement + generation 汇总，验证预算按 batch 汇总，交接预算按 operation 汇总。
不同错误交替出现不能重置预算，自动重试不能通过递增 generation 绕过限制。
人工重新授权可以开启新轮次，但保留累计历史。退避使用指数增长和 jitter，429 优先遵守
服务端重置时间；所有阶段还有 wall-clock deadline。Phase 1+ 放宽次数必须显式配置。

### retry_class

```text
transient   临时失败，自动重试（指数退避 + jitter）
recoverable 平台安全清理、停止后重启或对账重新投递，可在原阶段有限重试
fixable     需要 Agent 修改代码，可生成 CiRecovery（受白名单与次数上限约束）
blocked     需要人工介入，不自动重试
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
| git_push_rejected | blocked | 0 | 远端冲突/保护规则拒绝，人工处理，禁止自动 rebase |
| git_rebase_conflict | blocked | 0 | 人工解决冲突 |

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
| agent_stalled | transient | 2 | 杀掉当前进程并自动重试；重复失败后人工处理 |
| agent_protocol_error | recoverable | 2 | 停止旧执行组后按声明状态恢复，不复用不确定会话 |
| agent_process_lost | transient | 2 | 确认执行组静止，无声明才新建 Run |
| agent_completion_not_reported | blocked | 0 | 未声明完成，不能交接中间 commit |
| agent_approval_timeout | blocked | 0 | 请求过期，停止执行并转人工 |
| agent_input_timeout | blocked | 0 | 人工输入过期，停止执行并转人工 |
| agent_approval_denied | blocked | 0 | 人工决策或调整审批策略 |
| agent_max_turns_exceeded | blocked | 0 | 拆分 Requirement |

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
| pr_create_failed | recoverable | 2 | 查已有 PR 和分支；只有临时失败才重试同一操作 |
| pr_merge_conflict | blocked | 0 | 人工解决合并冲突 |

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

未知 `failure_code` / `retry_class` 一律 fail-closed：MVP 记为
`Failed + manual_action_required=true`；Phase 1+ 记为 `Blocked`。

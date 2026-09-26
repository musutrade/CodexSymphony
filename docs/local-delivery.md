# 本地 Git 交付（GH-105）

`local_git` 使用已登记的本机裸仓库，复用 Runtime、候选保全、完整验证、交付 outbox 和全局任务所有者。目标必须有已存在、直接指向 commit 的分支，符号分支和符号回执均不受理；当前实现支持 SHA-1 Git 对象。GitHub App、token、PR 和 CI 均不是此模式的前置条件。平台 PostgreSQL 仍是控制面依赖，不代表受管项目必须使用数据库。

## 登记与部署

仓库配置沿用 `/api/multi/repository`：`delivery="local_git"`、`remote="my-project"`、`base_branch="main"`，省略 `github_repository_id`。其余模型、项目、验证与预算策略仍由原评审接口冻结。

部署环境变量 `LOCAL_GIT_TARGETS` 指向平台拥有的绝对 JSON 文件。文件必须是普通文件，不能允许 group/other 写入；目标目录必须是 canonical 绝对路径的受保护裸仓库。registry、Git 元数据、交付回执及控制目录都应在 Agent 可写挂载之外。

```json
[
  {
    "reference": "my-project",
    "repository_id": 1,
    "repository_version": 1,
    "path": "/srv/symphony/targets/my-project.git",
    "branch": "main"
  }
]
```

`reference` 必须唯一，与仓库 `remote` 一致；内部仓库 ID、评审版本和目标分支必须逐项匹配。首次准备保存目录设备/inode、配置摘要和预期基线。更新评审版本后，管理员也须更新对应登记；旧任务不会继承新目标。

迁移 `0038_local_delivery.sql` 安装 `delivery:local_git` scope，默认关闭。通过既有版本化插件 scope 控制接口明确启用适用仓库。保留 `validation:native` 的独立 scope 检查。`/api/multi/repository` 的 `delivery_ready` 和 `capability_blockers` 显示本地登记、授权及分支检查；旧 `/api/repository` 继续保持历史响应格式。

Runtime 仍使用现有受审 launcher、准备适配器、验证 Plan 和 GitBroker。多仓路由的本地条目省略 `github_repository_id`，其 `remote`、`base_branch`、`version` 与登记一致。保留现有 `preparation.baseline` 配置字段；实际本地初始基线从已授权目标读取并导入 Broker，不能用部署配置的旧 SHA 覆盖目标。每次 Run 独立准备和绑定工作树。

## 更新与验收

1. 完整验证成功后，在同一事务创建本地 `delivery` 和 publish 意图，绑定原验证、候选及 target。
2. 提交前重核 scope、评审、预算、环境、候选证据、停止与保全，执行适用 `before_deliver` Hook，再持久化 unknown 尝试。
3. 禁用 Git Hooks、交互和网络协议，只从 Broker 导入候选对象。仅快进候选可更新目标，`--no-deref` 固定写入指定引用本身（[Git 语义](https://git-scm.com/docs/git-update-ref)）；Git `update-ref` 的同一事务比较原基线、更新目标分支并创建 `refs/symphony-deliveries/<operation digest>` 回执。
4. 回执是该操作已交付的事实，目标后来推进或删除分支也不会抹去历史版本。没有回执且基线一致表示未观察到提交；已有尝试的这种状态仍不授权盲目重发。冲突或未知保留原意图和任务占用，不强推、不回滚用户提交。
5. `Submitted` 后在独立工作树按原批准 Plan 验收实际交付版本。原子回执、完整候选身份、批准工具/配置、每项原始输出和子进程静止证据全部核对后才允许 `Done`。前后验证的 invocation 不共用。

验收进程使用现有 subreaper，持久化输入、结果、进程身份及停止证据。暂停、取消、scope/预算/存储变化停止该进程及派生进程；重启核对原 invocation，缺失原件不重跑。取消已经交付的版本只确认收尾，不撤销目标引用。工作树和验收证据使用现有存储预留；扫描发现本地 checkout 或验收输出超额时，持久化该交付的存储阻塞并停止验收，保留原件和任务占用。未知材料继续受保护。

父组完成事实使用 `platform-local-delivery/v1`，携带 `delivery_version`、目标、原交付和验收摘要；不生成 PR/CI/merged 字段。依赖及纯集成验证消费各仓库实际版本组合。只有原评审的 `linked-repair/v1` 路径映射和 `bounded_v1` 额度允许交付后关联修复：失败原件保留，新 Run 继承同一项/父组预算，在当前包含原交付版本的基线上修复，随后重新交付、验收并重跑适用集成检查。未批准范围、未知故障和耗尽额度继续阻塞。

详情中的交付观察和 `/api/requirements/{id}/delivery` 保留原动作、attempt、实际版本、验收和 blocker；本地响应使用 `mode=local_git`、内部 `repository_id`、`target`、`delivery_version`、`acceptance`、`storage_blocked` 和 `released`，省略 `pr_number` 与 `merged`。交付后的失败不会抹去实际交付版本。这些事实不依赖模型声明。手机交互与完整任务链体验分别由 #89/#90 承接。

## 迁移与回退

旧行默认 `github_pr`，原 GitHub worker 只读取该模式；本地模式使用明确的内部仓库字段，并保持 PR 为空。旧 GitHub 路径的行为和事实格式保持兼容。新的 mixed-version 序列化只对本地版本省略 GitHub ID。

回退前先暂停新领取，核对所有在途本地交付和验收已停止，保留数据库、裸仓库全部引用、Broker、工作树与 invocation 原件。仍有本地记录或混合父组时不能直接换回不理解这些记录的旧二进制，也不能删回执来模拟未交付。撤销 scope 阻止新的提交，既有事实继续对账；不会自动迁移在途任务到 GitHub 或重置累计额度。

## 证据范围

`apps/server/tests/local_git.rs` 使用临时裸仓库和独立 PostgreSQL schema，覆盖原子条件更新、目标/源码替换、丢失响应、取消、预算、scope、HTTP 登记、真实 Codex 进程编码、候选保全、本地交付、独立验收、父组版本组合和关联修复。真实 Runtime 测试的模型响应来自本机受控服务，准备准入使用独立 fixture；它不声称调用了线上模型、完成生产部署或代替 #122 的双路径现场验收。

质量要求仍是批准的 source-bound LLVM coverage/CRAP 先通过，再运行其余检查，最终精确源码树完整 Gate 通过后才发布。原始采集和 Gate 结果以本次开发保留的报告为准。#122 的组合回归、真实执行范围与尚缺证据见[双路径小任务验收](dual-path-acceptance.md)。

## AC 与验收入口

| AC | 保留的验证入口 |
| --- | --- |
| AC01 | `registered_local_project_runs_real_codex_and_reaches_done`：HTTP 登记/Ready、真实 Runtime 编码、项目验证、交付及独立验收；`product_validation_delivers_and_accepts_without_any_github_records` 核对原始证据与零 GitHub 记录。 |
| AC02 | `atomic_delivery_receipt_survives_lost_reply_and_later_branch_change`、`target_and_oid_substitution_is_rejected`、`non_fast_forward_and_missing_candidate_preserve_target`。 |
| AC03 | `lost_reply_reconciles_receipt_and_never_repeats_local_update`、取消及未知 invocation 测试，核对单次 update、原回执、进程静止和任务占用。 |
| AC04 | 暂停/scope/候选变化、未验证/失败验证、预算、登记版本、存储预留与超额停止测试；完整后端回归包含既有验证、保全和授权故障场景。 |
| AC05 | `local_completion_feeds_mixed_versions_and_pure_validation_without_empty_delivery`、原条目修复、混合集成修复后重验完整组合；既有 `github`、`automatic_merge`、`integration_validation` 回归。 |
| AC06 | 仓库读取与交付详情 API 断言、路由缺省 GitHub ID、模式条件返回及本文配置/迁移/回退；前端构建和完整 Gate 验证基础展示兼容性。 |

表中测试位于 `apps/server/tests/local_git.rs`（标明的既有回归除外）。它是验收入口映射，实际通过状态须以最终精确源码树报告为准。

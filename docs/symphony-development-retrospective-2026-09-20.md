# Symphony 开发 CodexSymphony 的断裂复盘（GH-12 ～ GH-25）

复盘日期：2026-09-20。本文基于本机 Symphony 运行记录整理断裂原因与优化建议，不修改综合方案的规范性要求或现有服务配置。前一次复盘见 [2026-09-13](symphony-harness-gate-retrospective-2026-09-13.md)，本文只覆盖其后的运行区间，并标注其中哪些 P1 仍在发生。

结论：5 天内 15 个 Issue 完成 14 个，但 Agent 实际运行时长只占墙钟的约 13%，其余时间在等人介入；Symphony 进程重启 57 次，几乎每次恢复都要手工修改交接台账。沙箱类断裂在切换可信开发环境后已归零；现在最大的耗损来自**验收依赖操作者在 Agent 之外完成的动作**，其次是**认证/上游失败直接烧掉 attempt 预算**和**CI 首轮失败率过高**。这三类都是 CodexSymphony 自身要处理的场景，应转化为产品规则与验收，而不是继续手工续跑。

## 1. 证据范围与统计边界

- 运行日志：`/home/gem/.local/share/codexsymphony/symphony/logs/log/symphony.log.1`，覆盖 2026-09-15 06:06 至 2026-09-20 02:06 UTC，73,863 行（debug 71,001 / info 2,547 / warning 171 / error 137）。读取时 SHA-256 `f8e3feebb7ad086f3b0f995104feb2883519efdd5ea3c204e29d87bce528ede3`。
- 交接台账：`/home/gem/.local/share/codexsymphony/symphony/WORKFLOW.lifecycle.md.handoffs.json`，读取时 SHA-256 `6bc41aa9504882009f030b90eee72bf7f3671f1904e60df080139b81b5f99ec1`。
- 恢复快照：同目录下 40 余个 `ghNN-*-recovery` / `ghNN-environment` 目录，以及 10 份 `handoffs.json.before-*` 备份、`resume_gh20_once.py` 等一次性脚本。
- 运行规则：仓库根 `WORKFLOW.lifecycle.md`（`max_turns: 12`、`max_run_attempts: 3`、`lifecycle_max_repair_attempts: 1`）。

日志按事件文本逐行计数，同一异常的堆栈重复行不重复计。`run_attempts` 多次被人工放宽，台账值是最终值不是完整重试率。本次未查询 GitHub 实时状态。

| 指标 | 结果 | 解读限制 |
|---|---:|---|
| Issue 数 | 15（GH-12～GH-25、GH-27） | 14 个 `done`，GH-25 当前 `blocked` |
| `run_attempts` 合计 | 48 | 配置上限 3；GH-22 用了 7 次，GH-20/GH-24 各 5 次 |
| Symphony 进程重启 | 57 | 按 `Running SymphonyElixirWeb.Endpoint` 计 |
| `Handoff rejected (agent_blocked)` | 21 | 其中 3 组 blocker 文本一字不差地重复出现 |
| Harness-Gate CI 失败 | 11 / 14 PR | 首轮 CI 通过率不足 30% |
| Agent 运行时长合计 | 15.07 小时 | 墙钟 116 小时，利用率约 13% |
| `total_tokens` 合计 | 258,582,568 | 输入 257.3M / 缓存输入 250.4M / 输出 1.30M |
| 单 Issue 最高 | GH-23：163.7 分钟、51.1M tokens | GH-24 31.4M，GH-22 25.6M |

## 2. 断裂原因分类

按次数排序。"状态"指本次复盘时该类问题是否仍会发生。

### 2.1 执行沙箱 / Runtime 环境不可用 — 7 次 block（已解决）

GH-17×3、GH-19×3、GH-21×1。原文包括：bubblewrap `needs access to create user namespaces`、`Read-only file system (os error 30)`、`codex-sandbox` launcher ENOENT、allowlist 内的 `index.crates.io` 被"local/private network addresses are blocked"拦截、嵌套 `command/exec /bin/true` 退出 101。

2026-09-16 #47 切换到可信开发环境（`codex-trusted` + `danger-full-access`）后这一类归零。代价是 GH-17 一个 Issue 在 09-15 15:32 被 block 后等到 22:46 才恢复，跨 3 次重启。

### 2.2 验收依赖操作者在 Agent 之外完成动作 — 7 次 block（未解决，当前最大）

GH-24×4、GH-25×3。原文：

- "Required real A01 setup not supplied: no designated disposable repository/smoke Contract or configured product Runtime"
- "Real A01 requires operator deployment of retained fixes to the external product"
- "A13 second repository and test-write authorization are not designated"
- "original product service has not activated the GH-25 routes (GET /api/multi/repository HTTP 404). The authorized original-service operator deployment is required"
- "OPERATOR-DEPLOYED.md requires a distinct supported operator action to release it"

模式一致：Agent 完成实现并通过本地验证，然后发现验收需要有人部署产品服务、指定第二个仓库或释放 Requirement 1。每次 block → 操作者手工做 → 重启 Symphony → 新 session 重读全部上下文。GH-24 累计 5 次 attempt、31.4M tokens；GH-24 09-19 15:55 block 后 22:38 才恢复。其中至少 5 次在开工后 5 分钟内就能判定前置不满足（GH-24 14:12 派发、14:15 block；14:33 派发、14:33 block）。

### 2.3 CI 失败但 Agent 拿不到原因；修复预算用尽后全部人工修 — 3 次 block + 8 次人工修复（部分解决）

早期 GH-13/15/16 三次 block 原文均为"Exact-SHA Trusted Harness-Gate failure exposes only a generic error; referenced host gate.stderr/run reports are not mounted in the sandbox, Actions has no artifacts"。装入 gate-diagnostics 后可见性问题解决。

但 `lifecycle_max_repair_attempts: 1` 用尽后，GH-20/21/22/23 的 CI 失败都由操作者本地修复并用 `resume_delivery.py` 续上（`gh21-ci-recovery`、`gh22-gate-recovery`、`gh23-gate-recovery`）。CI 失败根因记录：

- GH-23：`root_usage region coverage 8/11 below 80 percent`，未覆盖三条错误传播分支。
- GH-22：TypeScript risk probe 基线 JSON 归一化不一致（implicit else 坐标）。
- GH-21：本地探针 target triple 与宿主 `x86_64-unknown-linux-gnu` 不同。

共同点是 Agent 编写期间的自检与 CI 门禁不同身份、不同配置，CI 成为"发现"而不是"确认"。

### 2.4 模型 / 认证侧失败直接烧掉 attempt 预算 — 9 次 attempt 失败（未解决）

- ChatGPT OAuth refresh token 被吊销 2 次（09-15T22、09-19T09），共 27 条 `Failed to refresh token` 与 401，均在无人值守时段。
- `{:turn_failed, :upstream_retries_exhausted}`×2、`:response_timeout`×2、`{:port_exit, 1}`×3、`workspace_hook_failed before_run`×1。
- Symphony 对这些按 10s/20s/40s 退避重试。GH-22 在 09-19 08:59～09:12 的 13 分钟内失败 4 次、重启 4 次；GH-20 在 08:24 的 35 秒内失败 2 次。3 次 attempt 60 秒内烧光后进入 `agent run attempt limit reached; operator action required`。

这些失败与 Agent 行为无关，却与代码失败共用同一个 `max_run_attempts`。

### 2.5 测试 fixture 不稳定 — 3 次 block

GH-16 `Database proxy request rejected: 5`（PG proxy 握手超时）、GH-22 HTTP fixture 无 `RUNTIME_CONFIG` 无法采集持久化问答响应、GH-24 `recurrent PostgreSQL UnexpectedEof in delivery.rs:29 after test fixture recreation`。#49 项目内 HTTP 夹具已解决其中一类。

### 2.6 Symphony 控制器自身 — 约 20 次事件

- `max_turns: 12` 触顶：GH-12 连续 3 次、GH-13 连续 6 次到 `turn=12/12` 后"returning control to orchestrator"，每次开新 session 重读上下文，约 4 分钟一轮。
- `ENOSPC` 直接 `MatchError` 击穿 Orchestrator（`orchestrator.ex:580`，09-15 11:04）。
- `GenServer.call(WorkflowStore, :settings, 5000)` 超时 7 次导致 Orchestrator 终止。上次复盘 P1"配置读取异常不能直接击穿调度循环"仍在发生。
- 09-17 06:50 GitHub `nxdomain` 20 次，同时 `Skipping startup terminal workspace cleanup`；`workspaces/` 目前 22 GB 未清理。
- 每次恢复都是手工编辑 `handoffs.json`：57 次重启对应 10 份 `.before-*` 备份与多个一次性 resume 脚本。

## 3. 对 CodexSymphony 产品的优化建议

本项目就是同类编排系统，以上每一类都是它自己要面对的场景。以下按优先级排列，标注对应规则章节。

### P0：验收前置条件预检，不只是环境预检（对应 12.3、16、28）

现有预检覆盖 Cargo/node/DB 等环境项。应增加"验收输入就绪检查"：目标仓库指定、产品部署地址可达、GitHub App 安装存在、测试写授权明确。缺一项时**不启动 Run、不调用模型**，直接生成带明确清单的待办；`preparation_dependency_missing` 的语义足以容纳。2.2 中 7 次 block 里至少 5 次可以在开工前判定。

### P0：把"实现"与"需要外部动作后才能验证的验收"拆成两个任务（对应日常 V1 契约 M1）

GH-24 把"实现修复 + 部署 + 观察 A01"绑在一个 Issue 里，Agent 写完代码后只能 block。产品的需求拆分应默认把"验证依赖外部动作"的部分拆出，前半部分正常走 PR；后半部分等前置满足后再领取。

### P0：失败分类区分"环境/上游"与"Agent 失败"，前者不消耗 run_attempts（对应 28）

第 28 章已有 code 分类与 30s/120s 退避，需明确：401/refresh token 失效、模型上游错误、DNS、进程异常退出属于基础设施类，退避逐步走到 5 分钟并触发人工通知，不进入代码修复或 attempt 计数。这一条直接对应 2.4 的 4 连败。

### P0：操作者干预是一等 API，不是改 JSON（对应 4、10、28）

已有 pause/cancel/resume。还需：`授权一组新的基础设施重试`（第 28 章文字已有，须确认为接口而非改库）、`标记外部前置已满足并恢复`、`释放当前占用`（GH-25 当前卡的正是这个）。每个操作在"阻塞六问"页面上有对应入口，并记录为介入事件。

### P1：push 前用与 CI 相同配置身份跑完整门禁（对应 12、A06）

11/14 PR 首轮 CI 失败。产品的 custom gate 应在 push 前以与远端相同的配置身份跑一遍（`docs/quality/complete-local` 已有这套），使 CI 成为确认而非发现。"一次代码修复"预算用尽时，产品应导出完整诊断包（失败 step、原文 stderr、SHA、覆盖缺口）而不是只留 generic failure。

### P1：无进展识别（延续上次复盘 P1）

日志给出了具体形态：连续 N 次 `max_turns` 触顶且 workspace 无新 commit；同一 blocker 文本连续两次一字不差出现（GH-19、GH-24、GH-25 都有）。第二次相同 blocker 应直接转人工，不再启动模型。

### P1：配置/存储故障不能击穿调度循环（延续上次复盘 P1）

`ENOSPC` 与 `WorkflowStore` 超时在本区间仍发生 8 次。execution-control.md 的 tick 设计（不等待数据库查询、文件操作或模型调用）方向正确，应用 ENOSPC 与 DB 慢查询做故障演练验证。

## 4. 对当前开发流水线（`WORKFLOW.lifecycle.md`）的直接修改

这些不改变产品规格，只减少开发本项目时的人工介入。

1. `agent.max_turns: 12 → 40` 左右。已有 `max_runtime_ms: 4h` 与 token 上限兜底，无需每 12 轮丢一次上下文。
2. `before_run` 增加 `codex login status` 与 GitHub 域名解析的快速失败；更根本的是把 Codex 认证从 ChatGPT OAuth 换成 API key，两次 refresh token 吊销都在无人值守时段。
3. 发放 `symphony-ready` label 前先满足验收前置（部署完成、仓库指定），或在 Issue 模板中加"外部前置清单"，由 `before_run` 检查对应文件存在。
4. 清理 `workspaces/`（22 GB），为 Symphony 加磁盘水位监控；09-15 的 ENOSPC 即由此而来。
5. CI 失败且 repair 预算用尽时，自动导出 gate-diagnostics 到 workspace，使下一次 attempt 不依赖操作者手工拉日志。

## 5. 对实施队列的具体建议

- **0a 收尾（A13 前）：** 验收前置预检与"标记前置已满足"操作先落地，否则 A13 会重复 GH-25 的三次 block。
- **0a 故障演练补充：** 在现有 ENOSPC、只读 Git、准备超时之外，增加"认证过期 3 连败"、"同一 blocker 重复出现"、"验收前置缺失"三个用例，均有本文日志作为复现依据。
- **Phase 1：** 实现/验收任务拆分、push 前同身份门禁、基础设施重试授权接口。
- **继续后置：** 并行、公平调度、自动清理策略；本区间证据不支持将其提前。


## Codex 复核与处置（2026-09-20）

已核对当前开发控制器和操作记录：57 条启动记录、21 条 handoff rejected 可复现；
“模型运行时间约 13%”不能直接推出余下 87% 都是人工等待，其中包含 CI、停机及
无人派单时间。本文 GH-25 blocked 和资源待清理属于当时快照，GH-25 后续已完成。
GH-22 至 GH-25 的一次性 fixture 已清理并启用定时回收；产品数据库和验收证据保留。

优先修复当前 Elixir 开发 Symphony。实施了等待状态分离、基础设施独立有界重试、
受鉴权/幂等/版本校验的 issue 级恢复 API、宿主固定授权操作桥接和写盘失败暂停。
累计成本、原 PR 身份和正式 Gate 条件保留，开发模型为 gpt-6-astra low。
具体边界、操作方法和部署验证见 [恢复运行说明](symphony-operator-recovery.md)。

不采纳自动拆出未完成验收换取合并、放宽正式质量门禁、无界基础设施重试或
强制清空产品 owner。阶段预检只查当前前提，不要求尚未实现的路由提前存在。
对尚无精确宿主部署授权的请求，保留明确的 external wait；不把 agent 的任意
命令自动提升为宿主授权。未来新的部署仍需由操作方准备并登记具体固定动作，
而已登记授权的执行和续跑不再需要重复询问用户。

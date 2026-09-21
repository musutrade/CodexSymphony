# CodexSymphony

[M2 集成交付与上线清单](docs/m2-delivery.md)：同版本认证、HTTPS/执行隔离、手机接续、Bark 与恢复验收；区分受控通过、生产待验和 M3 范围。

[日常备份、升级保护与隔离恢复](docs/daily-recovery.md)：受限加密副本、停机一致性边界、只读恢复演练与管理员回退步骤。

[Bark 行动通知](docs/bark-notifications.md)：默认关闭、宿主凭据/网络隔离、持久去重和有限投递重试。

[手机接续与隔夜回答](docs/mobile-continuation.md)：同账号草稿/队列入口、持久问题、新 Run 恢复与暂停/取消验收。

[M2 HTTPS 与执行凭据边界](docs/m2-deployment.md)：部署配置、管理员安装/回退、源站检查与真实挂载隔离 fixture。

[平台账号与会话](docs/platform-authentication.md)：M2 登录、宿主账号管理、HTTPS／可信代理配置与受控验收入口。

[M1 使用、迁移与交付索引](docs/m1-delivery.md)：需求与队列的阶段验收、测试入口及 M2/M3 边界。

[未开始队列编辑与差异评审](docs/queue-editing.md)：合法重排不重评内容，变化项及依赖冻结后按精确版本重新授权，累计预算与完成事实保留。

[父 AC 覆盖与整组评审授权](docs/group-review.md)与[组依赖队列](docs/group-queue.md)：一次确认后按授权、依赖和仓库基线串行领取；缺适用验收能力时明确等待，不代表业务完成。

[自然语言生成草稿](docs/draft-generation.md) 与 Markdown/JSON 导入共用 Draft 模型；只有显式生成调用模型，草稿不授权编码执行。

个人 AI 开发编排系统：评审需求后由 Agent 编码，平台负责验证、PR、CI 和交接。首个日常版本的目标是正常路径自动到合并与业务验收完成，工作时用电脑，离开电脑后用手机接续。

当前已建立 Rust / Axum / SQLx / PostgreSQL 与 Angular / Material 应用，支持仓库登记/选择及需求创建、编辑、评审 Ready 和撤回，持久化不可变 Contract 与策略/预算快照。已加入 [执行控制与冷启动屏障](docs/execution-control.md)：持久化 Run、全局占用、后代进程监督及暂停意图。已接入锁定 Codex Runtime、累计额度和固定候选验证；已加入 [可靠 PR 交接与取消收尾](docs/delivery.md)。当前实施范围是 Phase 0a：localhost 上第一条需求到 PR，再验证多仓登记，始终严格全局串行。

[工作区与 Git Broker](docs/workspaces.md) 已实现本地独立 worktree、候选提交、完整工作保全和按阶段恢复；部分失败保留原件并阻断恢复。Runtime 与远端交付已接线；真实 A01 的部署条件和未执行边界见交接文档。

[GitHub App 预检与只读观察](docs/github-observation.md) 保存仓库能力及独立 PR/CI 事实，
缺能力或过期时拒绝领取；30 秒刷新、60 秒有效期及失败退避不启动模型。已接通条件 push、PR outbox 与精确合并事实释放；自动合并不属于 0a。

[执行环境预检与磁盘保护](docs/preparation.md) 已加入真实开发环境探针、持久化有限重试、
精确领取证据和存储停止保护；编码与远端写交接仍受后续任务门禁约束。

[需求累计额度](docs/budgets.md) 已加入首次授权冻结、跨 Run 调用预留、累计用量结算和独立等待计时；真实 Runtime 已使用该累计额度。

[Localhost 操作闭环](docs/operations.md) 提供需求表单、列表、详情/Run 时间线与待办箱，支持版本化回答和控制、独立事实展示、脱敏证据预览及持久化消耗/介入指标。分类存储、跨重试归并与容量保护见 [证据生命周期](docs/storage-lifecycle.md)。

[单仓集成验收](docs/single-repository-acceptance.md) 汇总 A01–A12 的复用测试、真实边界和部署恢复步骤。
真实 A01 已在授权仓完成到 Submitted/PR，证据见 `docs/quality/gh24/real-a01.json`。
[多仓登记与串行路由](docs/multiple-repositories.md) 已在第二私仓完成真实执行与 PR 路由，见 [A13 证据](docs/quality/gh25/README.md)；精确提交双 Gate 仍由独立服务验收，不能提前记作完整 0a 发布。

[M1-01 导入父子 Draft](docs/import-drafts.md) 提供确定性 Markdown/JSON 导入、来源与版本持久化；仅保存草稿，不授权执行。

## 从这里开始

| 文档 | 效力 |
|---|---|
| [综合方案 V10.4](Personal_AI_Software_Factory_综合方案.md) | 当前实施契约；第 23 章为队列，第 24 章为验收索引 |
| [日常 V1 契约](docs/daily-use-v1.md) | 已确认的多入口、父子队列、手机接续与自动交付要求 |
| [架构边界](docs/architecture-boundaries.md) | 三个真相分离，Harness-Gate 只提供验证证据 |
| [演进目录](docs/roadmap-specs/README.md) | 后期候选及启用条件，不构成当前开发/验收要求 |
| [本次范围收缩记录](docs/scope-reduction-2026-09-14.md) | 变更理由与迁移映射，不另定义行为 |
| [运行复盘](docs/symphony-harness-gate-retrospective-2026-09-13.md) | 历史事故依据，不覆盖当前规格 |
| [开发断裂复盘 2026-09-20](docs/symphony-development-retrospective-2026-09-20.md) | GH-12～GH-25 断裂分类与优化建议，不覆盖当前规格 |

## 已确认的日常使用场景

电脑/手机录入或导入 Agent 草稿 → 平台协助拆分大需求 → 一次评审整组 → 严格串行编码、PR、CI、有限修复、合并 → 子项与整体验收。普通澄清隔夜仍可手机回答后继续，仅变化范围需重评。最终纯验证项无需创建 PR；原范围内的合并后修复继续属于当前任务。

0a 到 PR 只是内部里程碑；日常交付以 [V1 契约和 B01–B08](docs/daily-use-v1.md)为准。

## 当前选择

- 一个 Rust 控制面 + PostgreSQL + 最小 Angular Web，30 秒只读 GitHub 刷新（60 秒有效期）。
- GitHub App；Agent 只通过受控工具提交/声明，平台经持久化 outbox 发布。
- 严格全局顺序覆盖编码、验证、交接、CI 等待和阻塞；具体释放条件见综合方案第 6 章。
- 采用 Symphony 的可信开发环境；普通命令和测试直接执行，网络声明仅用于依赖准备与连通性检查。
- 保留启动恢复闸门、工作保全、精确验证身份、0a 一次代码修复、跨 Run 累计预算和磁盘保护。
- 仓库先做交付能力检查；暂停保留占用，取消按完整收尾流程释放。旧 S2 合并字段推论已纠正，实际规则见综合方案 11.1/11.2。
- 手机、执行隔离、Harness-Gate 接入及自动合并随后分期；通用租约/诊断/资源/缓存框架按实际需要评估。

## 已有实验

[S1](spikes/s1/README.md)验证动态工具与工作区写边界；[S2](spikes/s2/README.md)及[补测](spikes/s2/README_S2b.md)验证 App、PR、Checks 与 SHA 守卫；[S3](spikes/s3/README.md)验证 Access；[S4](spikes/s4/README.md)验证 Gate 配置身份；[S5](spikes/s5/README.md)验证全局网络配置。
实验结论仅保留为历史记录。当前边界见 [可信开发环境](docs/trusted-development.md)：GitHub 与签名凭据独立托管，开发环境只接管本人可信代码。

参考：[OpenAI Symphony](https://github.com/openai/symphony)、[Harness-Gate](https://github.com/musutrade/Harness-Gate)。

## 开发环境与本仓库门禁

- `codex-version.lock` 固定 Codex 0.154.0；协议生成与兼容性检查绑定该版本。
- `harness-gate-version.lock` 固定 Core 0.4.5 与 Rust collector rc.6。独立源码／前端／合约插件见 `.harness-gate/collector-candidates.json`。
- 本仓库从开发阶段启用 Harness-Gate，CRAP ≤10，覆盖率 ≥80%；不改变未来平台对受管仓库的分期。
- [CI 范围与过期任务](docs/remote-gate.md#ci-范围与过期任务2026-09)：普通文档可复用同策略完整基线，过期 PR 检查自动停止；手动触发仍跑完整门禁。
- [本机开发](docs/local-development.md)、[完整门禁验收](docs/quality/complete-local/README.md)、[Symphony 启用](docs/symphony-development-setup.md)。
- `WORKFLOW.lifecycle.md` 供现有 Elixir Symphony 开发本项目；Agent 使用宿主 `github_api` 交付，不能执行 shell git push。
- 首个未来平台接管仓库为 `musutrade/disposable`，沿用 S2/S2b 的测试授权；与当前开发本仓库的 Symphony 验收分开。

Product Runtime deployments require a finite [storage lifecycle policy](docs/storage-lifecycle.md) via `STORAGE_CONFIG`.

开发控制器的有界重试、外部操作和受审计恢复见 [恢复运行说明](docs/symphony-operator-recovery.md)。
最新证据核对及下一项工作见 [2026-09-20 收尾记录](docs/quality/development-closeout/README.md)。
A13 原件恢复进展、剩余缺口及当前就绪状态见 [证据恢复索引](docs/quality/gh25/evidence-recovery.md)。

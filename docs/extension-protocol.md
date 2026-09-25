# 核心与扩展协议 v1

状态：统一规范，2026-09-24。本文唯一维护操作命名、版本和兼容语义。P1–P8 的最小类型与四类生命周期 Hook 已由 #103/#104 合入；P9 为 #118 的受控环境/验证/交付扩展契约。P10 区分实际调用点与待接入能力，不把类型支持当作运行能力已启用；不新建通用 RPC。

## P1 职责与公共身份

| 边界 | 核心保留 | 扩展提供 |
|---|---|---|
| 生命周期 hooks | 调用时点、授权、截止、保全与删除资格 | 项目准备、辅助产物和收尾结果 |
| 环境/验证扩展 | 冻结绑定、来源/完整性/身份核对、准入 | 环境事实、完整验证及逐项证据 |
| Agent 执行 | 队列、预算、当前执行身份、接受结果与验收 | 启动、事件、停止、可选恢复 |
| 交付 | 冻结策略、操作意图、预算、业务验收 | 提交/PR/CI 等实际交付事实 |
| 辅助决策 | 确定性权限与策略、是否采纳建议 | 结构化建议或不可用 |

所有调用绑定 protocol_version、requirement_id/revision、run_id（无 Run 的工作区清理可为空）、invocation_id、配置身份及资源身份。invocation_id 是核心生成的持久操作标识，恢复对账沿用；实际重试另记 attempt，不能制造新任务刷新预算。
工作区清理以 resource_id 和其身份代替当前 Run；不得随意借用已经终止的 Run 权限。
调用与结果都带相同身份；核心检查当前有效授权与运行代次，迟到结果可归档但不能恢复取消任务或授予后续动作。
版本不支持、结果缺字段/超大/类型错误均是可诊断协议错误，不自动解释为业务失败或全局存储损坏。

## P2 配置、权限和输入

部署登记允许的执行配置、脚本命令和能力；项目默认从中选择，需求覆盖只能选择允许项。评审时解析并冻结实际值，不在执行时重新读取可变默认。
模型配置含 provider/model/effort（不支持强度时为空）；hook 配置含名称、argv、适用角色、超时、输出限制和重放策略。无配置表示该 hook 不存在，不等同执行失败。
命令用 argv 执行，不拼接需求文本作为 shell。确需 shell 时固定已审查脚本入口，将不可信参数放在 JSON 输入中。协议版本与配置身份包括实际脚本内容/依赖包版本的可核验身份；不要求重建逐命令可信二进制清单。
可信脚本从已登记版本取得；Agent 改工作区脚本不会自动改变获授权 hook。执行时身份不符先停止该调用并给恢复条件，不偷偷运行新版本。
只提供必要工作目录、独立输出目录、任务上下文和获授权环境变量/项目测试凭据。模型、交付及签名凭据留在相应扩展的部署凭据提供方，不通过调用输入或日志传递；执行候选代码的验证进程不得带交付或签名秘密。继续采用现有可信开发环境，不声称具备恶意代码隔离。
超时、输出上限、重试和资源预算在调用前确定；适用时间/调用消耗进入现有记账，hook 内部自行调用模型不是免费路径，须走显式适配器及预算。

## P3 hooks 调用、结果与恢复

### P3.1 最小脚本封装

stdin 为一个 JSON 对象；stdout 为一个有界 JSON 结果；stderr 为有界诊断日志。长日志和产物放输出目录，通过引用返回。

```json
{
  "protocol_version": 1,
  "invocation_id": "hook-42",
  "attempt": 1,
  "requirement_id": 7,
  "revision": 2,
  "run_id": "run-9",
  "resource_id": "workspace-9",
  "config_id": "config-v3",
  "event": "before_run",
  "role": "coding",
  "workspace": "/work/project",
  "output_dir": "/work/outputs/hook-42",
  "deadline_at": "2026-09-23T12:00:00Z",
  "context": {}
}
```

成功结果含 protocol_version、invocation_id、attempt、config_id、status=success、artifacts；失败含同样身份、status=failed 和 error（code、可读 message、可选 evidence_ref）。角色枚举为 coding、repair、validation；事件/角色组合须在配置中声明。
artifacts 每项含输出目录内相对 path 和用途 kind；核心核对路径、文件身份及大小，再保存其引用。拒绝路径穿越和指向受限文件的链接；脚本返回的任意外部 URL 不自动可信或自动下载。
退出码为 0 且合法成功结果才算成功；非零、格式错误或状态不一致不视为成功。脚本不报告 authoritative timeout/cancelled/unknown，这些由监督器根据实际事实记录；脚本的 retryable 提示即使存在也不授予重试权。
不规定所有 hook 共享业务错误枚举；核心区分协议、环境、超时、取消及结果未知，保留原始证据。

### P3.2 调用时点

| Hook | 时点 | 失败影响 |
|---|---|---|
| after_create | 新工作区建立、身份持久化后，首次执行前；恢复已有快照不按新建处理 | 当前工作区未准备好，不能启动执行；保留供诊断 |
| before_run | 每次实际尝试启动前，初次/恢复/修复均适用；validation 仅运行明确适用的准备 | 阻止本次启动；按现有环境重试预算有限恢复 |
| after_run | 已启动执行组全部静止、核心保全完成后；正常/失败/取消路径均覆盖 | 辅助失败单独记录，不推翻已知编码/验收事实 |
| before_remove | 核心初步判定可删除后；调用结束再检查身份、引用和删除资格 | 失败或未知保留对象；不得绕过核心删除许可 |

before_run 失败且未启动 Agent 时不调用 after_run。after_run 可读封存材料并写独立辅助输出，不得改写候选；核心保全失败时保留阻塞，不让 hook 代替保全。
各 hook 有有限截止；持续失败的辅助 after_run 不永久占队列，但执行进程未静止或核心材料不安全时不得释放。清理失败不直接阻塞无关任务，真实容量不足仍由存储策略处理。

### P3.3 幂等与锁

开始前保存意图，事务结束后启动外部进程；外部脚本不得在业务数据库锁内等待。记录启动事实、最终结果及材料引用；优先复用现有记录，不强制新建独立状态机框架。
已确认完成的 invocation 不重放。崩溃后先确认旧进程组静止并对账结果；无结果不推断未执行。
默认 replay=never。仅声明且验证幂等的调用可在原 invocation 下增加 attempt 并有限重试；非幂等未知结果保留待对账/可操作待办，不能靠换 invocation 绕过。语义是有界重放与对账，不承诺任意 shell exactly-once。
超时和取消停止派生进程并确认静止；仅杀父 shell 不够。清理钩子不得删除自身所依赖的核心保全材料。

## P4 Agent 执行适配

最小逻辑操作为 start、observe、stop；resume 是显式可选能力。可直接包装现有 Runtime 方法，不要求独立网络接口。
start 输入冻结任务/模型配置、工作区、剩余预算、截止与可用工具；返回执行句柄。observe 提供 progress、usage、question、completion、failure 和执行结束事实，并绑定运行身份及可去重事件标识。适配器声明用量为增量还是累计，核心避免重复计费；缺失用量保留 unknown，不能当作零消耗绕过预算。
stop 是停止请求，不等于静止完成；由核心监督器确认子进程全部停止。completion 只是 Agent 声明，保全候选和独立验收之后才有业务结论。
能力至少区分恢复、取消、结构化事件、用量报告；首个实现复用 Codex。无法恢复时只在策略允许、已有保全的前提下开启新尝试，不冒充旧会话继续。没有可靠停止能力的执行器不接入无人值守执行。
候选/集成验证目前由既有独立路径完成；P9 的受审验证扩展可承接完整检查，不能改成模型自评。

## P5 模型选择与升级

第一增量只实现冻结的任务级配置；不同子需求可使用不同模型，同一活跃会话不热切换。规则顺序为合法的任务显式覆盖、项目默认；任何规则分类建议必须落到同一允许配置集合。
提供方不支持的 model/effort 组合在准备阶段明确报错；不得静默降级或换更贵配置。实际 API 接受配置的证据与返回模型标识在可得时保留。
后置升级要求：旧执行组静止且保全、已确认适用代码失败、当前授权允许该后继配置、次数和累计余额充足、未暂停/取消。环境、认证、存储失败不触发模型升级。
升级用新 Run/尝试关联同一需求与失败来源，继承项/父组累计消耗、修复次数、截止及权限。记录原模型、后继模型、原因和策略版本；不新建普通需求刷新预算。
交接输入含冻结需求、精确提交/保全快照、失败证据和未解决问题；不复制无限聊天，不假设跨模型共享会话。持续无进展按现有规则进入待办。

## P6 交付适配

逻辑接口统一为 delivery.capability_check、delivery.submit、delivery.observe、delivery.reconcile（见 P9）；旧草案 reconcile_cancel 归入 reconcile 的取消原因，不是新生命周期 Hook。核心持久化意图与操作身份，适配器执行后返回类型明确的事实；失败/超时/结果未知必须区分。
github_pr 复用现有 Broker、PR/CI 精确身份及合并对账；保留全部已有授权和独立验收语义。
local_git 由本轮 #105 实现：固定本地仓库身份、目标分支、候选提交和已验收交付提交。仅在预期基线一致时条件快进目标引用；响应丢失读取实际引用对账，分支漂移不得覆盖或强推。交付提交及依赖材料按核心保留策略可恢复。
本地交付不推送，不产生 PR/CI/merged 事实。目标引用更新后仍完成适用业务验收；失败后的关联修复沿用原授权与预算，不自动回滚用户新提交。GitHub 的“前置已合并”在本地模式对应“前置交付且验收完成的精确版本可用”，不能仅以 commit 存在满足依赖。
父组记录每个仓库的真实版本组合；三类事实保持分离，第三类可包含非 GitHub 的交付观察，不为不同模式制造虚假字段。
取消已发生的交付不伪造回滚；适配器只提供实际结果，核心决定收尾与队列释放。

## P7 可选决策接口

输入为最小脱敏状态快照、问题/schema 版本、允许选项及独立预算/截止；结果为 typed answer、可选概率信息、提供方/模型身份、可得用量，或明确 unavailable/unknown。
不同提供方的概率/置信度含义不得强行统一为“准确率”。模型建议既非事实也非授权，不修改预算、质量判定、业务 Done 或存储保护。
默认关闭；服务失败、超时或不完整输出只使建议不可用，确定性主流程仍可运行。#91 保留其独立授权/评测约束；本协议不要求现在实现通用模型决策服务。

## P8 兼容与协议验证

#103 已提供输入/输出版本、冻结身份、非法能力和旧配置映射契约测试；#104 已验证 P3 生命周期/未知副作用。#118 增加 P9 最小类型与兼容测试；#119/#120/#121 接入环境/验证/交付，#105 验证本地模式，#122 真实组合验收。#106 实际模型传参仍后续排期。
新增模式必须显式启用；旧运行继续其版本，未知版本拒绝新动作并保留材料。兼容升级优先增加可选字段；不能静默改变已有字段含义。
配置与模型凭据替换不等于任务扩大授权。迁移和回退范围、场景责任见改造需求第 5–6 节。

## P9 受控环境、验证与交付

### P9.1 版本、登记及环境绑定

协议主版本仍为 `protocol_version=1`。本次是新增可选的受控扩展配置与独立封装，不给旧 P3 JSON 添加必填字段，不改变 success/failed 的生命周期含义。未知操作、未知版本和未知字段拒绝执行；增加可选字段也须能力协商，不能要求旧严格解析器接受新字段。破坏含义或必填字段变化须新主版本，禁止静默降级。版本/操作枚举只在本文维护；脚本和 Rust 适配共享语义。

`ControlledConfig` 是原冻结配置的可选伴随配置：包含 protocol_version、environment 和 extensions。`ControlledConfig::freeze` 核对部署白名单后将全部类型化配置 JSON 的 SHA-256 返回为 controlled_config_digest，随调用一起冻结；既有 config_id 摘要算法保持不变。不能只冻结一个可变配置路径。#118 不更改现有持久化配置格式或自动迁移运行中任务。

environment 绑定 repository_revision（受管仓库的批准配置版本）、contract_digest（SHA-256）、host_profile_ref、role。契约引用项目实际工具/服务/限额/调度/变量白名单；不强制 Rust、项目数据库、容器或缓存。缓存关闭或命中统计不可得不使配置非法，不复用旧测试结论。平台数据库不是项目依赖。

每项登记含 id、implementation_digest、operations、scope_ref、config_ref、可空 credential_provider_ref。登记来自部署审查白名单，项目仅选择；能力/范围/配置引用不包含 App ID、installation、token 或私钥等厂商鉴权字段。登记及配置内容必须可按摘要恢复；Agent 不能替换实现、修改登记或扩大能力。具体供应商字段由扩展配置解析，未知必需能力在启动前明确拒绝。

环境核验发生于仓库启用、服务启动、执行准备、验证启动和交付前。区分批准契约、实际环境摘要及主机/临时目录等诊断；同角色 dev/CI 须匹配同契约，不把无关临时路径作为差异。工具/服务/资源或有效服务配置/进程版本变化使旧准入失效；受控安装恢复后重新核验，Agent 不升级宿主。

### P9.2 共同调用、来源和结果

共同 `Call` 含 controlled_config_digest 和 P1 的完整 identity（invocation/attempt、requirement/revision、run/resource、config_id）、operation、extension_id、implementation_digest、candidate(commit/tree)、environment_digest、policy_digest、deadline_unix_ms、required_checks。摘要使用 64 位十六进制 SHA-256，Git 对象 ID 按仓库对象格式由 Git 适配核验。environment_check 和 capability_check 可在尚无候选时传空 candidate；其余操作绑定已保全候选，交付后验收绑定实际交付版本。纯验证任务有源版本，无空交付。

`Call` 的最小 Rust 类型用于已有任务的执行阶段。仓库启用/服务启动尚无任务时，环境核验使用资源级封装：protocol_version、invocation_id、attempt、resource_id、controlled_config_digest、environment 绑定、extension_id、implementation_digest、deadline_unix_ms；结果回显这些字段并返回实际环境摘要、差异及证据。不能虚构 requirement/revision/run 或借用旧 Run 授权；该资源级执行封装由 #119 接入，仍服从同一来源、期限和失效规则。

invocation_id 是持久意图标识；一次真实执行增加 attempt，observe/reconcile 以自己的 invocation 关联原 operation_id，不能伪造新任务绕过累计预算。期限来自核心授权的最短有效期限；扩展不得延长。目标、预期基线、交付 operation_id、输入证据引用、工作/输出目录等操作参数同样冻结，参数摘要计入 config_id 或 policy_digest，不用可变目标替换已批准目标。

结果回显完整 Call；验证类 `Evaluation` 包含 verdict=pass/fail/unknown、checks（id、verdict、evidence）。证据引用含保全系统 artifact_id 和实际字节 SHA-256；长输出不内嵌，不能因脚本给出 URL 就下载或信任。输出沿用 P3 的有界封装和进程监督，Rust 实现无需启动外部脚本或通用 RPC。

来源不是 stdout 的自报字段：监督器绑定实际登记实现摘要、执行边界、输出通道、退出事实及已保存调用。仅接受该调用捕获/认证的结果，核心检查证据实际字节、可读性、大小和保留身份。远端验证的认证由受审扩展及部署提供方实现；核心核对其登记来源，不内置某厂商签名方式。`Evaluation::check_pass` 只作结构性前提检查，不证明来源、文件真实性、授权或进程静止。

只有当前授权有效、环境一致、候选及策略/实现摘要完全匹配、所有必需项有证据且 pass，才可提供该阶段准入。缺项、重复项、部分输出、无法读取证据、进程退出未知、超时/取消、非零退出或残留进程均不能提供 pass。fail 表示已确认检查失败；未知原因保留 unknown，不改写为代码失败或 AgentRun Failed。整体 pass 不能掩盖任何逐项 fail/unknown。

不适用由**调用前**批准策略确定：保存理由及策略摘要，将该项排除出 required_checks；扩展不能用运行时 N/A 替代必需项。空集合只在冻结策略明确允许且对应操作没有必需检查时合法，不能由缺配置推导。部分/迟到结果可保全归档；到期、取消、旧 attempt、旧 revision 或旧候选不授权新动作。实际操作在期限内发生但响应迟到的事实仍对账，不假装副作用未发生。

任一源码 commit/tree、未提交内容、环境、策略、实现、目标/基线、有效授权或证据字节发生变化，都使依赖它的准入失效。每次有副作用的操作前重核；已经保存的历史证据保留原身份，不删除、重贴新身份或复用为当前结果。

### P9.3 操作表

下表左侧为规范名；JSON `Operation` 使用右侧 wire 值。它们是扩展操作，四类生命周期 Hook 的 HookEvent 枚举保持不变。

| 操作 / wire 值 | 专属输入 | 返回事实 / 核心处置 |
|---|---|---|
| environment_check / environment_check | 环境绑定、核验时点、角色、期望环境契约 | 实际摘要、逐项 expected/actual 差异、诊断引用及 Evaluation；不符阻止进入该阶段 |
| validate / validate | 冻结候选、批准完整检查集合、无交付秘密的执行边界 | 全部检查及证据、pass/fail/unknown；核心核对集合和身份，不解释框架阈值 |
| before_deliver / before_deliver | 待交付操作/目标、有效 validate 证据引用 | 可选附加检查的 Evaluation；已配置则必须通过，不覆盖 validate 的失败 |
| delivery.capability_check / capability_check | 登记能力、目标/范围、所需操作 | 可用/不可用/unknown、范围及配置引用、证据；由扩展探测必要权限，不把 GitHub 条件强加给 local_git |
| delivery.submit / submit | 持久 operation_id、精确候选、预期目标基线、授权范围及准入证据 | 已提交实际版本/引用、确认未执行的失败或 unknown；超时不能推导未执行 |
| delivery.observe / observe | 原 operation_id、已知交付身份 | 最新交付事实、观察时间和来源证据；无副作用，不用旧缓存冒充新确认 |
| delivery.reconcile / reconcile | 原意图、已知事实、未知/重启/取消原因 | 已发生/确认未发生/仍未知及证据；取消不是已回滚，恢复授权由核心决定 |
| post_delivery_validate / post_delivery_validate | 实际交付 commit/tree、版本组合、适用业务验收集合 | 新 Evaluation，不能借候选结果替代交付后验收；通过仍由核心按业务策略决定 Done |

交付事实可复用既有 `DeliveryObservation` / `DeliveryOutcome` 并保存在原交付/恢复记录；GitHub 的 repository/PR/head/base/checks/merge 是该扩展的具体事实。local_git 只提供仓库/目标引用/提交/树和条件更新事实，不填空 PR/CI、不伪造 merge。Submitted 表示适用交付已提交，Done 表示适用业务验收满足；它们与准备/执行/验证/交付/交付后验收阶段分开，且均独立于 AgentRun。

GitHub before_publish/before_merge 位于适配器实际写操作边界，可使用同一受审调用机制，不是通用生命周期事件。完整验证和具体交付可以由固定脚本完成，无需另一套内置 Gate；凭据进程只能执行登记的不可被 Agent 修改的实现。候选代码验证始终不带交付/签名秘密，Agent 的 Apps/MCP、Git helper、SSH agent、宿主登录态及控制目录访问须由部署真实隔离并在启动/恢复路径核对。仅去掉变量或提示词约束不足以保证秘密隔离。

外部操作先保存意图再执行；未知结果先 reconcile，确认未发生且冻结策略允许、预算足够时才能重试。任意脚本不承诺恰好一次。停机/取消先监督整组静止并保全，不把 after_run 成功视为验证或交付成功。复用单协调器、现有验证/交付/恢复记录；无插件市场、动态加载、额外事件总线或每概念一张表。state_changed 按后续真实需求单独接入，不是本轮前置或本版可执行操作。

## P10 实际调用点、迁移和验证边界

| 当前代码入口 | 当前行为 / 后续接入责任 |
|---|---|
| `extension_contract.rs`、`project_hooks.rs::register/event/after_run/before_remove` | 已实现 v1 冻结配置和四类 Hook；#118 保持格式/事件兼容，不令 after_run 承担验收 |
| `environment_service.rs`、`preparation_service.rs`、`coordinator.rs::start_runtime/recover` | #119 已接入可选冻结环境绑定；启用、服务启动、准备/恢复/修复及 Agent 启动前核验 |
| `validation_runner.rs::execute_cancellable`、`validation.rs::verify`、`validation_store.rs` | GH-120 复用候选/步骤账本，环境绑定任务经 validation_context / validation_hook / validation_supervisor 执行完整 P9 validate，核对结果来源，不用 Agent 自报 |
| `delivery_worker.rs::tick/reconcile/publish`、`delivery_control.rs`、`github_delivery.rs` | 现有 GitHub outbox/对账；#121 接入受控交付操作及凭据边界，#105 实现 local_git |
| `merge_prevalidation.rs`、`merge_dispatch.rs`、`merge_validation.rs`、`merge_acceptance.rs` | 当前 GitHub 合并前检查及合并后验收；#121 对应适配器写边界和 post_delivery_validate，不移入通用必填 PR/CI |
| `controlled_contract.rs`、`environment_probe.rs` | #118 通用类型与校验；#119 补齐资源级封装和受审环境探测监督、任务 Call/Evaluation、环境观测迁移，GH-120 已接入候选 validate；#121 交付扩展调度仍后续实现 |

旧 Repository 继续通过 `ExtensionConfig::from_legacy_repository` 映射既有 Codex/GitHub 默认；未配置受控扩展不是新能力自动授权。已有冻结任务、累计预算、失败证据和在途交付意图均保留。未来评审显式选择并冻结新配置；替换实现/策略必须重新评审和验证。回退先暂停新领取、静止并保全、对账副作用；采用能读取已有记录的版本，未知能力拒绝新动作但保留材料，不能删记录来绕过恢复。#118 无数据迁移，回退代码不改变现有记录。

#118 测试 `extension_contract` 与 `controlled_contract` 覆盖旧配置映射、未知版本/操作、未登记或被篡改扩展、无 GitHub/无 Rust/无缓存配置、缺项/重复/过期/错身份结果。它们是受控契约验证，不是无 GitHub 的产品闭环；#119–#122/#105 分别提供实际环境、可信执行边界、可替换验证、交付和真实任务证据。本仓库完整 Harness-Gate 要求不因产品协议可替换而改变。

## P11 仓库环境执行接入（GH-119）

Repository 的 `environment` 可选；缺省保留历史行为。值为 `environment::Plan`：
HTTP/Repository 快照将 Plan 编码为 JSON 文本，读取时严格反序列化；宿主文件和
探测协议使用原生对象。操作详情的 report 也使用既有观测 JSON 文本约定。
`controlled` 复用 P9 ControlledConfig，另含 `host_profile_digest`、`extension_id`、
既有 `lockfiles` 相对引用、`roles.dev/test`、`ci` 和可选 `cache`。
role 包含 expected（精确观测值）、services（安装/配置摘要）、runtime（仅 runtime.*
字段的批准可变值）、checks 和 dev/test 差异说明。环境内容摘要覆盖这些声明及
宿主 profile 摘要；完整方案摘要还覆盖登记和绑定。评审存储方案而非重新读取默认值。

宿主 Registry 的 profile 固定登记实现、可执行路径、资源目录、超时和 approved_plans。
profile 身份覆盖登记、路径、资源根、超时及证据根，排除批准方案列表以避免循环摘要。
项目只能选择批准值，不能安装/升级宿主。Registry 本身是受审宿主输入，不是工作区文件。
产品登记的 scope_ref 为 `repository:<内部 ID>`，环境 repository_revision 为
`repository:<内部 ID>@<仓库版本>`；任务从原评审快照核对两者，不能跨仓借用批准。
资源级独立 CLI 不创建这些业务记录。证据根与项目资源根不能重叠。

探测 stdin/stdout 的类型为 `environment_probe::Request/Response`。Request 包含
P9 ResourceCall、调用时点、角色、完整方案摘要、资源根、适用工作目录和 required_checks；
任务核验还包含通用 Call，绑定原 requirement/revision、冻结扩展配置、策略/参数摘要。
该 Call 的 run_id 对应**实际探测监督器**的 RunKey；这是基础设施执行事实，不创建
或冒充 AgentRun。没有任务的启用/启动/CLI 检查只使用资源级身份，不虚构任务身份。

Response 回显整个 Request，返回 actual、逐项 CheckResult；任务调用必须同时返回
与通用 Call 精确匹配的 Evaluation。资源级结果不具有任务验收权限。
`actual` 证据引用解析为调用目录的 `actual.json`：UTF-8、无附加换行的确定性 JSON，
核心重新序列化核验 SHA-256、落盘并保存 actual_digest。环境扩展的声明不能替代
登记实现摘要、捕获通道、退出状态及完整静止回执。输入限 64 KiB、各输出流限 1 MiB，
超时最大 600 秒；超限或失去静止证明拒绝准入，未知意图阻止新的环境执行。
准备阶段进一步取原准备台账剩余期限；已证明没有启动的调用保留 not-started 回执，
不能把真正缺少进程静止证明的调用标作未执行。

每个声明服务必须观测 installed、process、config、effective_config、syntax_valid、
semantic_valid；前两者匹配安装摘要，中间两者匹配批准配置摘要，后两者必须为 true。
因此更新了磁盘配置但运行中服务仍使用旧配置不能通过。工具、镜像、资源及调度值
由项目受审实现探测，核心比较 expected/actual；不内置 Rust/数据库/容器要求。

缓存缺省关闭；启用的 identity、scope、capacity_bytes、writable 和可选 seed
进入冻结方案。scope 绑定独立宿主资源根，平台校验容量/权限，种子只接受受信预置、
只读且内容匹配清单的版本。不可获得的统计为 unknown，不适用由批准方案决定；
缓存和统计都不能生成测试/覆盖率验收结果。操作与迁移见[环境接入说明](repository-environments.md)。

## P12 完整验证接入（GH-120）

[验证 Hooks](validation-hooks.md) 说明受审 Plan 的 P9 适配、进程来源证明、交付准入、
访问边界、迁移及回退。四类生命周期 Hook 保持不变。验证入口不执行交付，
项目检查可使用通用 `validate` 选择器；原检查类型及冻结任务保持兼容。

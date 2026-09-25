# 项目完整验证与交付准入（GH-120）

GH-120 在既有候选验证账本上接入 P9 `validate`。项目选择管理员维护的
Runtime repository route 中的 `validation` Plan；Plan 指向工作区外的受审入口，
包含实际文件 SHA-256、完整逐项命令、超时及是否允许代码修复。入口可以运行项目
脚本或调用批准的验证服务。核心不解释覆盖率、CRAP 或测试框架内部规则。
本仓库仍执行 `.harness-gate/QUALITY.md` 的完整 Harness-Gate，不能用示例脚本替代。

## 使用和身份

新项目在已有 Repository environment 绑定中选择经宿主批准的环境契约，评审
`policy.allowed_checks: ["validate"]`，Contract 的每项使用 `check: "validate"`。
该通用检查名要求环境绑定；`cargo_test` / `npm_test` 的历史语义保留。步骤 ID
必须与批准 Plan 完整对应。`/gate-entry` 是兼容保留的参数占位符，实际执行的是
受审 `Plan.entry`，不要求安装名为 Gate 的程序、Rust、npm、项目数据库或缓存。

`validation_context` 从冻结 revision、保全 manifest（含基线）、批准 Plan 和独立
环境观测构造 Call。受审进程适配登记名为 `reviewed-process-validation`，登记摘要
来自部署拥有的 Plan，不从 Agent 输出解析。契约与实际环境分别保存；策略摘要
覆盖 revision、manifest 和 Plan。来源证明是核心启动的进程与静止收据，不要求
项目提供签名服务。外部服务包装器须按其批准方案核验远端证明，不能只转发 PASS。

执行前确认原 AgentRun Succeeded、整组静止和候选保全；恢复出的 Git commit/tree
及非忽略的未提交内容经独立 Git 检查。验证前后重新探测项目环境。输出保存在
平台验证目录的原始日志中，Call、逐项摘要及 Evaluation 与原 candidate_validation、
validation_step 一起保留。stdout 中的 PASS 字样没有授权作用。检查缺失、重复、
非零退出、未知退出、错身份、摘要不符或迟到都不能进入交付。

当前进程适配的调用期限上限为一小时；已有任务期限更短时取较早期限，逐项超时仍以批准 Plan 为准；期限保存在
原 Call，重启不会延期。恢复只消费原调用，不重新运行未知调用。supervisor 的
启动身份有十秒等待上限；停止证明等待二十秒后仍不确定则保持阻断。失败时保留
raw stderr/stdout、已产生的步骤日志和调用记录，未知结果分类保存在原 failure 字段，
不改写成功 AgentRun，也不刷新修复预算。

## 准入与访问边界

初次发布、修复更新和既有自动合并的实际写入口复用与宿主无关的 `validation_context::admit`；既有 outbox 由 `delivery` 包装适配。
其他交付扩展只需提供冻结任务和 commit/tree，不传 PR/CI。
它重读当前环境、部署 Plan、候选 checkout、调用身份、原始步骤、Evaluation 和
整组静止证明；原 outbox 的暂停、取消、撤权、源版本及预算检查继续在发送前执行。
配置了 P9 证明的记录缺失证明不能降级。暂停/取消触发持久失效标记，恢复运行不会
复活旧证明。验证不发布；验证成功也不等于 Requirement Done。

具体隔离仍由受审执行适配及部署提供，参见 [M2 部署边界](m2-deployment.md)。
管理员必须将验证入口、配置、控制状态和秘密放在候选视图之外；只过滤变量不够。
本次核对的产品路径如下：

| 路径 | 共同核验位置与边界 |
|---|---|
| 初次 Agent、用户回答后的恢复、暂停后恢复、代码修复及关联修复 | `runtime_client::execute` → `coordinator::start_runtime`，每次重验冻结环境并创建独立 codex-home；通用 start_reserved 也执行启动环境准入；没有复用旧 app-server |
| 完整候选验证 | 独立 supervisor → 受审 Plan.entry；不提供 Runtime home、平台数据库 URL 或交付/签名凭据 |
| 既有组合/合并前后验证 | 保留原监督、批准 Plan 和版本集合校验；交付写入口额外核对原候选 P9 准入；扩展式 post_delivery_validate 仍归 #121 |
| Apps/MCP、宿主登录、Git helper、SSH agent | 部署只给 Runtime 所需模型身份；不得在新 codex-home 配置有写能力的 Apps/MCP。宿主 HOME、Git/SSH/Apps/MCP 配置及登录态不挂载，SSH_AUTH_SOCK 不传入；环境扩展核验实际 launcher、配置和服务身份 |

真实 namespace fixture 分别执行 coding/validation 入口，证明普通项目读写可用，
控制、GitHub、签名、通知及模拟宿主 HOME 不可见；后者包含 Git helper、SSH 和
Apps/MCP 配置哨兵。这是受控部署边界证据，不宣称完成生产部署或对抗任意恶意代码。
未按批准 profile 配置部署不可据此获得安全保证。独立执行 UID 仍属 0c。

## 验证、迁移与回退

`tests/validation_runner.rs` 使用真实 Git 候选及两种受审入口（Shell、Python），
验证无 Harness-Gate 的完整检查、恢复、替换实现、缺项、过期及原始结果损坏。
`tests/environment.rs` 的 P9 场景执行真实子进程和一次性 PostgreSQL，保留可消费
Evaluation，验证 PASS 后源码/实际环境/入口改变、静止证明缺失、暂停后恢复、
进程失败和运行中暂停。原有 Runtime、修复、交付及合并回归继续执行。

迁移 0033 仅给 candidate_validation 增加伴随证据、required 和失效标记，并安装
控制失效触发器；历史记录默认维持原协议，不伪造环境或 P9 证明。新环境绑定任务
首次调用才冻结受审 Plan；已有 Call 永不替换。未配置 environment 的旧任务沿用原
批准策略和记录。旧失败、累计预算、在途副作用均不删除。

回退先暂停领取、确认所有 supervisor 静止并保全，按原 operation 对账未知交付。
保留 0033 和所有证据，只切换能识别 hook_required/失效状态的兼容版本；不能启动
忽略新准入字段的旧发布 worker。被暂停或过期的证明须经原恢复授权建立新的验证
尝试或新评审 revision，不能清除失效位、改写成功证据或借新 ID 增加预算。

真实 GitHub/local_git 两种交付小任务仍由 #121 → #105 → #122 串行完成。本项不执行
生产写入、云服务 spike、合并或业务验收，也不把开发控制器能力描述为 Rust 产品能力。

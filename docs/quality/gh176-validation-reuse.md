# GH-176：验证输入与候选登记

本项修改现有 publication host 和 remote admission；不修改 Rust/LLVM 测量语义、质量阈值或受保护分支检查。开发顺序与验证顺序仍由 [AGENTS.md](../../AGENTS.md) 决定。当前默认部署继续使用精确提交的完整 Gate；下面的新路径只有独立审查、验证并安装后才启用。本文件不是部署完成或成本验收报告。

## 两种身份

完整 Gate 的原始 ledger record、report、requests、capture context 永远保持原执行提交。已安装的 `codexsymphony-validation-contract/v1` 描述完整验证的依赖，安装入口以摘要固定该文件；工作区不能提供或更新此授权。它必须由审查者确认依赖枚举完整，而不是把 `complete_dependency_review` 布尔值当作自动审查证据。

契约绑定批准身份、runtime/config/trusted files、固定基线、Git 读取的已批准 reader 与分类、实际外部依赖目录/文件、必需载荷和有效期。外部输入必须枚举实际读取的依赖（包括忽略的 node_modules、Cargo 源码/配置等）；仅锁文件不能证明这些实际字节没有变化。不支持的读取、遗漏或未知依赖应拒绝启用该契约，继续完整验证。Git `input` 读取进入输入身份，变化即重新执行完整 Gate；`label-only` 只用于提交标签，`source-selection` 只支持与保留清单绑定的 tracked 文件枚举。声明的 reader 摘要必须来自已有批准；审查者还必须核实读取的用途和完整性。

新协议先要求已提交、干净的候选，且实际 source tree 等于 HEAD tree；dirty/untracked 或隐藏的非提交源码变化不支持复用，在昂贵阶段前返回阻塞。输入身份包括源码字节与完整文件权限清单、tree、干净状态、tracked 选择、环境指纹、契约摘要、外部依赖实际字节/权限和 Git input 读取。执行 checkout 的分支相关输入目前不支持，必须先扩展审查契约才能启用。实际变化不会被“相同代码”豁免。复用还要求最新匹配 ledger PASS 仍有效，且源绑定后端 measurements、摘要、原 report 与审查声明的载荷均保持原摘要；后续 FAIL、revocation、未完成 attempt、过期、缺载荷或来源被篡改都不能命中。旧部署未保存契约输入的 PASS 不能作为新协议的复用种子。

新提交的独立 `codexsymphony-publication-binding/v1` 作为现有 pin 的独立消费者保存，引用原 validation ID 与原提交；原 Gate 回执不改名、不覆写，publication receipt 只指向当前绑定。它重新绑定精确 head 和当时 main。绑定的完整字节在保存前由现有 records budget 计费，到期或释放后不再准入；原始 native 数据不获得永久保留。如果契约要求的载荷已被清理，复用被拒绝，按现行流程重新验证。相同输入的第二次调用只检查身份、有效性、载荷和提交准入，不启动昂贵 capture。

## 冻结前后

安装的 publication settings 绑定远端配置与契约。冻结前在宿主临时 bare repository 中 fetch main（不读取候选的 URL rewrite/credential/filter 配置，不修改候选 refs），核验祖先条件、host/config/trusted 输入、固定基线、remote verify-only 模式与普通候选登记能力；已知不兼容会在 capture 前失败。正常候选本身的具体审计在完整结果产生后登记，不能提前伪造 validation ID。预检不锁定 main，也不保证后续网络、容量或远端准入成功。

冻结后再 fetch 并复核 main；变化时明确返回阻塞，不 merge/rebase，不自动再次 capture。已完成的质量 PASS 保留在 ledger；发布阻塞不伪装成新的质量 FAIL。后续显式恢复重新检查输入契约，按真实依赖决定复用或完整验证。阶段开始/结束、状态和墙钟写入现有 publication `timings.jsonl`；receipt 给出 reuse reason。一次调用只有一次 capture 调度，没有自动重试环。

## 普通候选登记

remote host 经独立审批安装 `--verify-only --candidate-registration` 后，读取现有 pin 中的候选审计，不把每个 tree 固定在服务配置里。一次宿主软件升级仍需正常审查/安装流程；该部署不是 Agent 的普通候选写权限。

操作员通过已安装 release 的 `register_candidate.py --config <installed-config> --reviewed-audit <independently-reviewed-document> --head <exact-head> --expected-base <main>` 登记已有 v3 audit。入口校验已批准 runtime、standing ledger PASS、保留 evidence、审计绑定和远端精确 head/tree/main，登记末尾再检查 main。注册只写已有有限保留 pin，并计入同一 records budget；重复相同登记幂等，冲突/并发拒绝，迟到结果释放 reservation。登记不写批准文件、不替换工具、不调用安装器/systemctl，也不导入候选模块。

登记和本地提交绑定 pin 都是独立消费者，不能作为已确认 PR 的 merge proof；remote sweep 保留它们至显式 release 或 expiry。CI 仍读取当次精确 PR head、最新 main 与 Actions attempt，核验同一已安装输入契约、实际外部依赖及有效载荷，并要求该 head/base 的独立本地绑定。head/parents 等提交敏感输入由认证远端元数据重新计算，失配拒绝准入；tracked/status 仅支持已绑定的干净提交清单；本地使用安全的 index diff 与实际 source tree 对照，避免 porcelain status 执行候选 clean filter。仍需要真实确认的 PR proof 才能准入 merge。策略、工具、权限或测量语义变化只能走独立兼容发布，不能走普通登记捷径。

部署入口为审查后的 `tools/install_remote_gate.py --verify-only --candidate-registration --validation-contract <reviewed-contract> ...` 以及 `tools/install_publication_gate.py --validation-contract <reviewed-contract> --remote-config <installed-config>`。这些安装器不由 feature task 自动执行。没有新契约时保留旧精确提交规则；没有登记能力时启用新 publication 配置会失败，不静默降级为未审查复用。

## 验收证据边界

`tools/publication/test_validation.py` 使用真实 Git fixture、真实 ledger 与 fake Gate 调度，验证不同 commit 相同完整输入零新增 capture、原证据不变、输入变化失效、dirty 候选提前阻塞、commit-sensitive 失效、有效期、缺载荷、撤回以及 main 更新不自动合并。`tools/remote-gate/test_verification_inputs.py` 验证独立提交绑定、外部依赖变化、旧契约、缺载荷和提交敏感检查；`tools/remote-gate/test_evidence_admission.py` 验证有限候选登记、幂等、服务生命周期调用被禁止、策略失配、并发与迟到拒绝。

fixture 的零 capture/零新增 native 字节不是现场成本对比。完整 Issue 验收还需要同一已审环境的旧链/新链墙钟、真实 capture 次数/native 新增字节/人工介入，以及核心和 remote Gate PID/start time 前后读回，并在最终精确候选通过受保护 CI。没有现场证据前不声明 #176 完成，不新增 GH-90 业务模型调用。

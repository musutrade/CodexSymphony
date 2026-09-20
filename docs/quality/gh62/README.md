# GH-62 / M1-03 AC 与证据映射

基线 `1abf239ec7e667501b464faa7cc2a2ce28bef564` 已包含合并的 PR #60、#66。
本项只交付评审、原子授权与持久组队列；调度、validation_only 执行和最终父级业务验收仍未实现。

| AC | 实际检查 |
| --- | --- |
| AC01 | `group_review.rs` 真实 PostgreSQL 持久化；`e2e/group-review.spec.ts` 在桌面和手机评审三个 code_change 与最后 validation_only，展示精确 revision、仓库策略、修复范围及预算。`browser_acceptance.py` 停止 API 后真正重启，比较本组内容、评审、授权快照、队列、账本及绑定仓库。 |
| AC02 | `domain_checks_coverage_versions_policy_and_budget` 与 `postgres_rejects_invalid_coverage_references_plans_policies_and_order` 覆盖缺父 AC、悬空/过期引用、缺验证、依赖缺失/环/非法顺序、越权预算及仓库撤销/失效；`postgres_atomic_review_replay_races_and_persistent_accounting` 在预算与授权 INSERT 之后注入队列触发器异常，确认全部回滚。 |
| AC03 | 同请求并发重放返回相同授权；`competing_confirmations_create_one_authorization` 的不同并发请求只允许一个成功，另一个 409；版本冲突和 request_id 输入冲突不重复授权，组队列保持唯一。 |
| AC04 | 项及组累计已用量/在途预留跨新 revision 和重新授权保持；不足时整组拒绝，允许的新上限只改 limits。检查 Run、legacy Ready、owner 不因授权变化。旧 `/api/requirements` 及 `/api/multi/requirements` 的 Draft 身份仍拒绝绕过评审。 |
| AC05 | 桌面 Chrome / Pixel 7 真实 API 浏览器操作、键盘保存与一次确认、刷新读回、axe、横向溢出、减少动效及强制颜色模式；界面区分“已授权 / 等待调度能力”和“验收完成”。旧单需求与全局控制回归仍运行。 |

## 证据边界

- 原始日志、浏览器截图/读回、真实 API 响应、LLVM 与 TypeScript 测量保存在工作区 `artifacts/gh62/`。
- `.symphony-evidence.json` 以 SHA-256 声明实际文件；宿主归档与回执由控制器负责，Agent 不声称已经归档。
- 本地检查不签发正式 Gate。精确发布提交的 Harness-Gate 与 Trusted Harness-Gate 仍由独立宿主验收；未更改可信摘要、门禁配置、覆盖率或 CRAP 阈值。
- A13 历史原件缺口继续保留 [已有记录](../gh25/evidence-recovery.md)，没有以新测试补造旧证据。

## 预检与验证中的修正

旧固定端口冲突已由提供的 E2E 工具改为空闲端口并核对进程所属。
首次复测在旧测试库中遇到重复草稿/仓库，重建指定的一次性 test fixture 后基线 20 项通过；合成持久 dev fixture 未重建。
全局单实例锁会拒绝同时运行的后端进程测试与浏览器 API，因此实际进程验收串行运行。
重复全量测试遗留的 storage 控制状态曾阻止旧预算用例的预留，覆盖率复测前重建 test fixture；失败日志保留。

新增代码保持固定 collector 支持的普通函数体；未修改 collector 来规避不支持的语法。
浏览器重启比较保留完整的本组事实及批准仓库快照；实时仓库总列表可能包含其他测试刚登记的仓库，不能把这类正常新增误判为本组重启丢失。

最终命令与测量汇总见工作区 `artifacts/gh62/validation-summary.json`。

桌面与手机截图人工复核了标题层次、文字换行、主要操作和错误反馈；未见页面横向溢出或遮挡。配色沿用已批准的公共设计令牌，axe 的适用对比度检查通过。组输入控件高度至少 44px，按钮沿用公共至少 40px 的目标，状态使用文字而非仅靠颜色表达。

## 本地质量结果

最终固定源码测量覆盖 1108 个 Rust 函数和 131 个 TypeScript 函数，要求的覆盖率/CRAP 违规均为零。Rust 使用真实 LLVM 导出与固定 source-risk collector，TypeScript 使用原源码插桩副本与固定 collector；未修改生产源码为插桩版本，也未使用另一提交的测量结果。契约采集包含 84 个实际 operation/status 响应，测得 breaking_changes=0、client_drift=false、compatible=true。

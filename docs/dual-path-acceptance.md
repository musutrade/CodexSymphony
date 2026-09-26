# 双路径小任务验收（GH-122）

本项在 #105 合入后的 `9edde80f5938fd48b3bdac7f3148e3bb19125185` 上实施。#102/#83 的排期是 #105 → #122 → #106 → #89 → #90。本次交付同时携带开发流程、开发盘回收修复和 Codex 0.157.1 升级。所有验证遵循 [AGENTS.md](../AGENTS.md)，历史通过记录不充当组合树的发布依据。

## 当前证据边界

`apps/server/tests/local_git.rs::javascript_local_delivery_accepts_two_reviewed_check_implementations` 补充实际 Node 进程、Git 原子更新、完整验证和独立交付后验收的组合回归。两个独立 fixture 使用不同的受审检查实现，各自执行行为和语法两项检查，核对原始输出、实现摘要、实际交付版本、Done、零 GitHub 记录和单次交付。候选项目没有 Cargo、npm 依赖、GitHub 或 Harness-Gate 配置。测试数据库属于平台控制面。

该回归的候选由测试构造，使用既有 `npm_test` 检查类型和宿主 Plan，实际运行 Node 检查脚本；通用 `validate` 类型另须受审环境绑定。它不调用线上模型，不证明真实环境准备、网络隔离、缓存关闭配置或 GitHub 外部交付。既有真实 Codex 进程测试使用受控模型响应，也不等于线上模型验收。两条真实链已另行执行至 Done，实际回执见 [本次证据目录](quality/gh122/README.md)；受控回归与线上运行的证明范围分别记录。

## 小任务执行范围

两条任务串行执行，每个独立验收环境只放行当前小任务。初始范围为每条逻辑任务 20,000 tokens、10 turns、600 秒模型工作；真实调用触达 token 上限后已经停止并保全。用户随后明确选择追加方案，每条逻辑任务的累计上限调整为 200,000 tokens、10 turns、1,200 秒模型工作。追加通过原账户的版本化授权完成，实际用量及未知调用预留均保留，不通过新环境重建额度。确定性产品缺陷的修复后重验须先停止并保留旧环境，绑定新源码及环境，并把旧、新尝试的实际消耗计入同一上限。未知外部写仍须对账原操作，不能通过新尝试绕过。

| 项目 | GitHub 路径 | local_git 路径 |
| --- | --- | --- |
| 目标 | `musutrade/disposable-2`（ID `1377749969`） | 新建独立 JavaScript 裸仓库 `gh122-javascript` |
| 开发任务 | 添加一个小型纯函数及相应行为测试，保持既有 Rust 项目完整检查 | 实现 `value()` 返回 `after`，补行为测试和语法检查 |
| 基线 | 2026-09-26 只读核对 main 为 `b7527479849506046b7ad9b74466db7b2a65a3cb`；执行前重新核对并冻结 | 宿主创建初始版本，登记 canonical 裸仓库、main、内部 repository ID/version |
| 项目环境 | 沿用项目锁和批准检查入口 | Node、无第三方包、无项目服务，明确 `cache: null`、`ci: false`、无 GitHub/Harness-Gate 配置 |
| 验证 | 项目完整检查；保留精确候选和受审实现身份 | 同一核心流程分别绑定两种检查实现；每次都执行行为、语法检查 |
| 外部写上限 | 一个任务分支、一个新 PR、通过原保护后一次合并；不改 workflow、规则或保护 | 一个目标 main 的原子快进及对应回执；不配置 GitHub 凭据/API |
| 完成事实 | 产品观察精确 PR head 的 `test-job`（Actions App 15368），合并并验收实际合并版本 | 实际交付 commit、原子回执、独立验收结果、Done，重启读取原事实 |

已核对 GitHub main 保持 strict required check、管理员强制、禁止强推和删除。历史 GH-121 runner 的授权为 `merge=false`、`main_write=false`；不能用它执行上述新合并范围。新 runner 必须绑定当前产品源码、二进制、配置及本表的单任务范围。不得将宿主 `gh pr merge` 代替 Rust 产品的合并流程。

交付相关的 CI 观察由产品按冻结策略完成。开发代理不启动额外轮询或后台 watcher；用户通知 CI 结果后继续开发交付环节。

## AC → 执行与证据

两条真实链及重启持久性检查已通过。以下 Rust 测试位于 `apps/server/tests/`，受控故障可复用完整 Gate 的同源码执行结果，不必重复线上破坏试验。

| AC | 受控入口 | 真实证据要求（回执见本文末及证据目录） |
| --- | --- | --- |
| 01 | `delivery.rs`、`automatic_merge.rs`、`github.rs` | 当前产品 Runtime → 验证 → PR → 精确 CI → 合并 → 实际版本验收 |
| 02 | `local_git.rs::javascript_local_delivery_accepts_two_reviewed_check_implementations`、`registered_local_project_runs_real_codex_and_reaches_done` | 线上模型编码、正式环境准入、无缓存/项目数据库配置、无 GitHub API、独立本地交付至 Done |
| 03 | `validation_runner.rs::two_reviewed_hooks_produce_consumable_p9_evidence_without_harness_gate`、`retained_results_cannot_replace_approved_commands_or_omit_checks`、`protected_plan_rejects_tampering`；`validation.rs::rejects_every_identity_and_evidence_failure`；`local_git.rs::pause_scope_revocation_changed_candidate_and_unknown_send_block_delivery` | 实际候选、Plan/实现摘要、每项原始输出、环境/策略绑定；替换检查实现的独立批准记录 |
| 04 | `environment.rs::installed_new_binary_does_not_approve_an_old_running_process`、`real_two_project_admission_detects_drift_and_preserves_isolation`；`delivery.rs::lost_push_and_create_responses_reconcile_without_duplicate_work`；`local_git.rs::completed_acceptance_reconciles_original_invocation_after_restart`、`cancellation_stops_acceptance_and_reconciles_before_releasing_owner` | 本次运行进程的二进制和配置身份、恢复原 invocation 的记录 |
| 05 | `extension_lifecycle.rs::same_candidate_plugin_upgrade_revalidates_without_erasing_failure_or_budget`、`real_plugin_crash_timeout_protocol_and_quality_failure_preserve_distinct_facts`、`repository_scopes_filter_before_outbox_and_recheck_revocation`、`notification_replay_extends_once_without_resetting_attempt_history_or_business_state` | 单任务仓库 scope、验证代次、业务与通知状态独立记录 |
| 06 | `operators.rs`、`local_git.rs` 中交付 API 断言 | 两条任务的实际 API 记录；无缓存/编译/CI 的不适用与 unknown 分别记录 |
| 07 | 原 runner 的 timings、验证/交付/模型调用台账 | 开始/结束、实际模型调用、tokens/耗时/未知项、人工介入原因、源码/环境/配置和交付 SHA |
| 08 | 本文与真实结果回执 | 完成后更新 #102/#83/#90；不宣称父组、手机、全部通知渠道或生产现场已验收 |

#126 尚未合入时，完整失败诊断保全与授权读取仍由 #126/#89/#90 承接。本项不得声明其已通过。

## 本次前置与回退

首次真实 local_git 尝试在模型启动前暴露了环境探针竞争：Coordinator 的恢复探针与 Runtime 的启动探针共享证据根，后者将前者尚未写出静止回执的正常执行误判为未知；Runtime 又已先写入 session，后续 worker 按禁止重放规则跳过了该 Run。该尝试的 Requirement、Run、错误、数据库备份和环境探针原件全部保留，任务已暂停、服务已停止。记录确认 `model_call=0`、`delivery_attempt=0`；这不把缺失启动回执的旧 Run 追认为完成或静止。

修复在进程内串行化环境探针，使对账发生在上一探针结束之后；跨进程遗留的未知执行继续阻断。Runtime 仅在环境及权限准入通过后、实际派生进程之前登记 session。新增并发 launch/recovery 回归核对两份独立真实探针回执；启动前拒绝回归核对零 session、零模型消耗，并在修复受控配置后沿用同一 Run 成功启动。修复后的源码须重新取得测量及检查结果，不能沿用修复前 2,211 个函数的 PASS。

修复后的独立环境已真实启动模型，首次调用累计输入 27,038、输出 131、cached 13,184（input 的子集），超过原定 20,000 tokens；产品停止并保全了原 Run，未发生验证或交付。调用结算仍不完整，未知预留不能因为进程静止而释放。当时原任务暂停，运行和数据库原件保留；之后已按显式用户授权追加原账户预算并完成任务。补充的宿主 `budget increase --stdin-json` 命令复用原有授权事务，独立检验累计用量、未知预留、停止/暂停意图及幂等行为，不通过新 Requirement 清零。

现有常驻 A01 API 使用 `a01-gh24/releases/storage-scope-7c592b0/codexsymphony-server`，不能证明当前重构版本已部署。不要直接升级该旧任务服务或复用其在途数据库；为本次验收绑定独立环境，保留既有进程、任务和预算。

Codex 升级候选的 `environment.lock.json` SHA-256 为 `ffec8c8a60675b67b978f21ccce882e3faecc5fc02fb5291bce389d9d2eece6a`，现有 Gate 宿主仍绑定 `1f84fb20cf81d6285eb97b1aae737afdc538e05579ef519aa7dc7e4d53367972`。与原宿主批准相比，受信输入仅这个锁文件变化。任务范围批准已固定新版本，采集器、阈值、签名边界和 required checks 保持；最终组合树仍由发布前完整 Gate 证明，回执在宿主及 PR 引用，不写回被验证提交。

远端 Gate 还固定 `WORKFLOW.lifecycle.md` 的摘要。本次仅将开发顺序归入 AGENTS、保留控制器适配和 frontmatter；发布前须对该明确变更审核并更新绑定。不能先推送再等待可预见的 protected-file 失败。

Codex 升级改动涉及的 Python 生成器、环境探针和手机 fixture 已获用户明确批准，采用回收修复同版本的任务范围 Python 测量方案。该批准仅适用于此次三个入口，不是永久扩大采集器策略。历史升级证据见 [原记录](quality/codex-0.157.1/README.md)，它不包含 #105 或本次组合回归，不代表最终树通过。

本次补充 `tools/tests/test_codex_upgrade.py`，验证新增可选能力只改变 schema 身份而不改变消费字段、准备阶段拒绝旧 Runtime、手机 fixture 的握手与续跑。fixture 的输入循环归入 `main()`，便于对实际入口取得独立行覆盖率与复杂度；未改变脚本执行协议。coverage.py 7.16.1 / Radon 6.0.1 的精确源码测量已通过：`generate` 为 15/15 行、CRAP 4；`tools` 为 15/15 行、CRAP 7；`main` 为 24/24 行、CRAP 9。三项采集测试通过，采集前后源码摘要一致。原始证据保存在 `.harness-gate/reports/gh122/python-upgrade-01/`，授权范围另见同级 `authorization.json`；完整组合树仍须最终 Gate。

## 真实运行与恢复记录

local_git 在原任务追加预算后完成交付：实际 main 为 `89f7680384b35ebb740a2068e76490a7fa5cc074`，tree 为 `57d86dc98412b35f627fdbca016458cca0be604c`。实际 Node 行为测试及语法检查通过，独立交付后验收通过，Requirement 进入 Done；重启后仍是两次 Run、两次模型调用、一次交付，GitHub 仓库/PR 记录均为零。两次调用的 usage.complete 均为 false，已观测 input/output 合计 83,497 仅为下界，cached 是 input 子集；未知预留继续保留，Done 不等于账单最终结算。

GitHub 路径的线上模型生成候选 `d4fca58879b78f997ebde0b98317c2287642fd1e`，tree 为 `ae06b53b6d1ece44f75cf4c606a0f0e7c529cb2b`。Cargo 两项实际测试通过；独立 marker 断言编译、执行均退出 0，但宿主配置漏写成功输出，旧退出码适配器依约判为 unknown。此时未创建 PR，也没有交付尝试。该调用已观测 input 87,539、cached 70,784、output 586、模型工作 40 秒；结算未完整，150,000 tokens/1 turn/600 秒预留仍保留。

这次配置错误同时暴露恢复边界缺口：旧格式 unknown step 没有结构化恢复事件，普通 revalidate 也不改变已固定的 post_merge 计划。本次补充 [明确绑定交付验收的恢复决定](lifecycle-recovery.md)，在原 Requirement、修订、候选及账户上创建后继验证；新脚本仅在原断言实际成功后报告结果。原计划、全局 GitHub 策略及保护规则保留，不通过新 Requirement 重置额度，也不改写旧验证结果。新恢复代码已取得独立源绑定测量及检查结果；后续恢复与实际合并回执见下文。

后继验证成功后，真实交付又发现核心存储汇总仍假设一个 Run 只有一条验证：保留旧失败及后继成功记录后，标量子查询返回多行，容量检查因数据库错误拒绝继续。该时刻发布尝试为零，错误尚未到达 GitHub API。存储汇总改为保留完整验证列表，并确定性选择未被替代的当前验证；候选及 PR 绑定也使用同一选择规则。新增真实 PostgreSQL 回归反复扫描原失败与后继交付，检查失败及原始证据未改写、PR 身份不漂移。修复后的源绑定测量、检查及真实恢复结果已另行取得，先前失败及零发送事实仍保留。

存储修复后恢复原操作时，暂停已使后继验证失效，发布继续被正确拒绝。原流程缺少该状态的受限重验入口。本次增加零发送交付的显式重验和审计化验证引用更新；旧成功结果仍为成功但保持失效，不把 resume 当作恢复旧验证。涉及发送记录、未知状态或已存在 PR 的操作拒绝此路径。新代码完成源绑定测量及检查后，已在原任务执行该受限恢复。

验收中暂停/取消通过产品控制接口执行，保留 PostgreSQL 控制面、Broker、候选、所有 invocation 和本地交付 refs。未知外部写只对账原操作，不能盲目重发。回退配置必须先确认任务进程静止；已交付版本不能通过删回执或重置引用撤销。旧版本不理解本地交付记录时禁止直接降级复用该数据库。

## 完成结果与证明边界

GitHub 原任务已通过受保护交付：[disposable-2 PR #12](https://github.com/musutrade/disposable-2/pull/12) 由产品创建并以 squash 合并，实际合并 SHA 为 `3e7befaf0b5bdc835854cc1d5a14c62865aec4c4`。产品观察精确 head 的 CI，执行独立合并前验证，再按明确批准的后继计划验收实际合并版本，最终为 Done。原 workflow、保护和全局策略保持。一次线上模型调用、一次原交付操作、push/create 各一次、一条验证引用更新审计；旧 unknown、暂停失效及两代恢复记录全部保留。

最终二进制 SHA-256 `57887784dd747647cf7cdc1197ed99e9823f9ddc075c2fb2c26ab42e03389ae1` 完成 GitHub 恢复、发布、合并和实际版本验收；线上编码由回执中记录的较早二进制完成，不能宣称全部阶段都在最终版本上从头运行。local_git 的最终版本检查为原在线交付的持久性读取。两条原任务都已在最终二进制重启后核对 Done 和不增发，隔离服务已停止，控制面与回执保留。

[最终 Rust 源码测量](quality/gh122/rust-measurement-summary.json)覆盖 2,223 个生产函数、373 项测试／41 组：行和区域覆盖率至少 80%，CRAP 不超过 10；随后格式、Clippy、编译通过。全部 287 项源码输入绑定见证据目录，失败重做及人工介入没有从时间索引删除。这些事实不代替最终精确提交的完整 Gate，也不宣称未知调用预留已结算、全程无人介入或父组全部完成。

最终发布检查曾因预算测试中假凭据未采用明确占位写法而失败，完整质量测量虽通过，整体仍记录 FAIL。只修改测试占位值及对应不泄漏断言，生产二进制摘要保持完全相同；随后 source17 测量、格式、Clippy、编译与独立秘密扫描通过。该次失败和重做时间见 [证据目录](quality/gh122/README.md)，发布继续要求新的最终精确树完整 Gate。

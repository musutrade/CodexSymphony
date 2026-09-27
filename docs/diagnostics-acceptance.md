# GH-126 验收证据索引

开发与发布顺序只由根 [AGENTS.md](../AGENTS.md) 定义。下面是实验到 AC 的对应关系；实际结果由保留的日志、源码绑定测量和宿主凭证决定，不以测试名称代替通过证明。

| AC | 实验与核对内容 | 源码入口 |
| --- | --- | --- |
| AC01 | 实际 shell 非零退出的完整长日志、Python 的独立 Markdown 和大单行 JSON；经产品 Runtime 工具逐段重建，核对首尾失败、长度和导出摘要 | `apps/server/tests/diagnostics.rs::real_multilanguage_reports_are_read_through_product_tools_and_repair_scope` |
| AC02 | 实际验证失败，沿原有授权恢复入口绑定修复 Run，由固定版本 Codex Runtime 读取原失败的完整报告；另一次实际失败 after_run Hook 的报告经产品读取路由重建，保留原 Run 失败和工作区身份 | `extension_lifecycle.rs::approved_adaptation_uses_real_codex_runtime_and_preserves_fault_identity`；`project_hooks.rs::after_run_requires_quiescence_and_preserved_snapshot_but_auxiliary_failure_keeps_fact` |
| AC03 | 实际成功/业务失败/非零退出、启动错误、崩溃、畸形结果、超时、取消、输出上限；数据库写入故障原子回滚，不伪造 available | `diagnostics.rs::subprocess_failures_limits_and_atomic_capture_failure_preserve_truth`；既有 `project_hooks` / `validation_runner` 用例 |
| AC04 | 持久数据库重新连接、并发重复读取、迟到 attempt 不覆盖、候选身份隔离；独立本地 HTTPS 实验真正停止并启动服务器进程，复用会话继续读取并核对全量摘要 | `diagnostics.rs::reconnect_concurrent_reads_late_attempts_and_historical_revocation_keep_scope`；任务外 `restart_acceptance.py` |
| AC05 | 任务/Run/修复关系、暂停/取消/撤权、历史修订仓库授权；路径/URL/符号链接/替换/修改/导出篡改；压缩和二进制明确拒绝；先完整脱敏再分页，含跨行及嵌套 JSON 敏感字段 | `diagnostics.rs`；`tests/unit/diagnostic_capture.rs`；`tests/unit/diagnostics.rs` |
| AC06 | 实际低配额、实际数据库容量策略、过期保护、过期删除与累计分配保留、删除失败回滚；UI 显示 missing 与原因且不提供下载 | `diagnostics.rs::quotas_expiration_identity_corruption_and_pagination_are_truthful`、`installed_storage_policy_and_actual_database_capacity_limit_capture`；`web/angular/e2e/operations.spec.ts` |
| AC07 | shell/Python、显式受审的普通命令和 v2 反馈，不依赖 Gate/GitHub；桌面/移动 HTTPS 详情页查看、下一段、完整下载摘要、WCAG 和无横向溢出 | `tests/support/diagnostics.rs`；`web/angular/e2e/operations.spec.ts::views retained diagnostics and downloads the complete verified export on this viewport` |
| AC08 | v1/未知协议与旧选择器拒绝、失败 artifacts 必填、过量与不安全引用拒绝；现有环境/交付/存储/恢复回归；升级、回退、配额与读取文档；最终完整源码绑定 Gate | `extension_contract.rs`、`extension_lifecycle.rs`、`environment.rs` 等回归；[诊断协议与运行说明](diagnostics.md) |

证据类别必须区分：shell/Python/Hook 实验执行了真实的受控子进程；Codex Runtime 实验执行固定版本的真实程序，但 Responses 来自本地受控 fixture，不表示外部模型或生产现场验收。浏览器和进程重启实验使用隔离数据库中的明确 SQL fixture，验证真实受保护产品接口和 UI，不作为实际生产者失败证据。真实失败生产者证据由 AC01/AC02 的独立记录提供。本次未部署、清理旧任务、发送外部消息或调用付费外部模型。

## 保留的本地结果

以下路径保留在本机任务证据区，属于本地实验记录。最终完整 Gate 的源码树、环境、报告摘要与 PASS 由仓库外安装宿主持有的 `publication/GH-126/receipt.json` 决定；本索引不充当发布凭证。

- 后端 LLVM 捕获根：`~/.local/share/codexsymphony/gh126-development/measurements/backend-20260927T072537`。原始对象、计数器及输入清单在 `probes/backend`；实际重新导出的测量在 `probes/backend/measurement-20260927T074651/measurements.json`，SHA-256 `32f8a52bce62e33b53cd9141731f0a5b8224c03722b8ecf4aa511cbc926753ba`。2303 个生产函数全部满足现有覆盖率和 CRAP 策略，`changed_inputs=[]`。
- 前端原始源码计数器根：`~/.local/share/codexsymphony/gh126-development/measurements/frontend-20260927T080213`。安装采集器的实际响应在 `tmp/codexsymphony-ts-risk-2ybwbp/collector-response.json`，SHA-256 `12f50255c08a7ec38d4e35ff0f5844fd63c388734026cb48090e4b399da4fb55`。23 个可执行文件及 202 个生产函数全部通过现有策略；声明文件依安装宿主的既有分类保留，未增加排除。
- 测量 PASS 后的检查按 `timings.jsonl` 记录执行。后端 format / Clippy 已在 `gh126-development/checks-20260927T074816` 通过，随后 Rust 输入没有变化。最终前端 lint / build、密钥扫描及产品验收使用 `gh126-development/checks-20260927T080312` 的只读源码副本。

下表路径均相对后端捕获根的 `tmp/`；这些是实际实验输出而不是仅列出测试名称。

| 实验 | 保留记录 | SHA-256 |
| --- | --- | --- |
| 多语言真实生产者与 Runtime 工具逐段读取 | `diagnostic-fixture-cd25802e-3a71-4c01-8d62-20be786589ce/tool-read-evidence.json` | `05f5a0a70aecc486f855c22e11c2a52362839939460981ff625d17d75e8c5c67` |
| 持久数据库重连、并发读取、迟到 attempt 与历史撤权 | `diagnostic-fixture-3ef7fc8f-a232-414b-aff8-e73cf63233d5/restart-concurrency-evidence.json` | `ef96a4d193c9627583655b67ca966b32813e26fca5e71a8129e5ad45736a788e` |
| 子进程各失败状态与真实持久化故障 | `diagnostic-fixture-a39cc949-f660-4078-be7b-915d0316b5ba/failure-state-evidence.json` | `d8fc5d273a238f7aeafed842d20145110b225734320d2f55e3908ff52bf9edbb` |
| 原恢复入口授权的真实 Codex Runtime 读取与修复 | `validation-runner-9b1f15f1-e09c-43aa-b4f2-8d9e33852088/authorized-repair-diagnostics.json` | `ba9486984d1e0ef8883c8ec78f99c26f1ca930a6552e84e255b663532e2b4ebd` |
| 实际失败 Hook 与候选身份、完整产品读取 | `project-hooks-2e3f6008-1339-46c2-8b31-b7a622adb328/failed-hook-diagnostics.json` | `862328b5d331a213ce093fd4ba639e50670a87e030b9d3297680b8745f96710e` |
| 真实 Runtime stdio 工具调用与完整摘要 | `runtime-fixture-6f3014e3-b0e1-47b6-878d-2121cfc1cb65/diagnostic-transcript.json` | `62ac4a46a662f6d7007f18a293a1610e83d2f2daaa6115bbfc759d25642246ee` |

产品验收根为 `~/.local/share/codexsymphony/gh126-development/checks-20260927T080312`，以下路径均相对该根：

- `workspace/artifacts/gh71-product-contract/validation.json` 和 `client-measurement.json`：108 个真实 HTTPS 响应变体通过，既有合约无破坏，实际生产 HttpClient 清单无漂移；原始凭据只在内存验证后被遮蔽并重新校验。
- `browser-recovery.stdout`：桌面／移动共 30 个产品浏览器回归全部通过。诊断用例验证分页、首尾失败、完整下载、缺失原因及禁用下载、WCAG 和横向布局。
- `workspace/artifacts/gh126-browser/report.json`：为保留内存 attachment，另外只运行了桌面／移动的两个诊断用例（均 PASS），使用现有 Playwright JSON reporter；每个下载均为 190021 字节，摘要 `af8b9374ccb50470f63acc5603aa470d448bf9887fdcb16d2380d61fd3448c16`，与 manifest 一致。截图在 `workspace/web/angular/test-results/diagnostics-evidence/`。
- `workspace/artifacts/gh126-restart/receipt.json`：真实服务器 PID 6 → 34（隔离 PID 命名空间），两次正常退出；同一会话继续读取、同一 manifest、8 路并发；25 段完整重建 204027 字节，摘要 `9b1f7555f94a1c67eee4043cdea6dada5975aeb8880dc8b5136410bf4e11261e`。
- 各阶段开始时间、耗时及 PASS/FAIL 在根 `timings.jsonl`；失败和恢复原因在任务证据区 `~/gh126-measurements/readiness/recovery.jsonl`。手工 namespace 的 PATH、输出目录和数据库客户端问题保留了失败日志；生产源码修改后重新测量，没有把失败作为 PASS。

可携带的摘要及对应原始记录 SHA-256 在 [本地实验记录](../artifacts/gh126/local-acceptance.json)。完整原始对象、LLVM 计数器及源文件证据保留在捕获根，完成的缓存依安装宿主登记规则管理。最终 Gate 在所有源码、说明和本记录冻结后执行，结果以宿主持有凭证为准。

## 最终 Gate 前端测试恢复

候选 `980b76e` 的 `run-2369a69b492e` 完整质量测量通过，但 `frontend.tests` 因测试配置缺少 Node 类型而编译失败，故整体 FAIL，未发布。已在测试专用 `tsconfig.spec.json` 引用已锁定的 Node 类型；生产行为、测试断言、依赖和质量策略不变。

修复后，`frontend-20260927T092511` 的原始源码覆盖率与 CRAP 重新测量 PASS（23 个文件、202 个生产函数），后端捕获输入逐项核对未变；`checks-20260927T092552` 的 52 个 Angular 单元测试、lint、生产构建及密钥扫描全部 PASS。失败原件、恢复阶段时间和新测量摘要见 [恢复证据](../artifacts/gh126/frontend-gate-recovery.json)。这些检查不能代替修复后冻结树的新完整 Gate 凭证。

## Python 示例的任务限定测量

用户于 2026-09-27 明确批准 #126 对 `examples/project-hooks/project_hook.py` 使用 coverage.py 与 Radon 的任务限定替代测量；不构成 Python collector 的长期批准，不改变现有策略。工具包按原下载摘要核验后解压至新采集目录，执行完全相同源码的只读副本。全部可执行行及两个函数行覆盖率均为 100%，函数 CRAP 分别为 4、6，6 个采集用例全部通过。涵盖失败附件和调用身份、真实 Git 检查/状态报告、数据库命令边界、四类事件、协议入口成功/失败与旧/未知协议拒绝；数据库边界使用受控替身，没有生产部署或外部写入。

原始 coverage 数据、逐行/分支结果、源码与工具包摘要、授权原文、采集脚本和日志保留在 `gh126-development/measurements/python-20260927T093233`；摘要与精确函数结果见 [Python 测量证据](../artifacts/gh126/python-hook-measurement.json)。此记录仅覆盖本次 Python 修改；最终发布仍要求整个冻结树的新完整 Gate PASS。

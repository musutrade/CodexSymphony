# 单仓集成验收与有限试用

本入口复用当前 0a 的测试与产品部署，不建立第二套故障矩阵。A01 真实贯通、确定性故障测试、
开发宿主验收和精确提交双 Gate 分别记录。GH-24 已在原 Requirement 1 完成真实 A01，产品创建 [测试 PR #4](https://github.com/musutrade/disposable/pull/4)，详见
[集成验收记录](quality/gh24/README.md)。A13 未实施，不能宣称完整 0a 发布。

## 普通环境与可重复入口

准备锁定 Rust/Codex、Node/npm、PostgreSQL 16、psql、Playwright 浏览器、可写构建/执行/证据目录。
网络声明只描述依赖。连接失败、缺工具或容量不足在编码前报告，不消耗代码修复次数。
产品的 App 凭据在外部控制服务，正式 Gate 密钥在独立验证服务，二者不注入开发环境。

```sh
cd web/angular
npm ci --offline --no-audit --no-fund
cd ../..
# TEST_DATABASE_URL 必须指向可丢弃数据库；已有可信环境只需用其 URL 注入 wrapper。
python3 tools/single-repo-check.py artifacts/gh24/local-NEW
```

入口先检查实际工具、端口、目录、数据库连接和浏览器启动，再依次运行原 Cargo workspace
测试（包括真实锁定 Codex transport）、fmt、Clippy、续观脚本测试、前端 lint/unit/build、
原桌面/手机 E2E、Gate config/secrets/audit。每项实际命令和退出码写入新目录；失败即停止，
不覆盖历史。已安装依赖通过普通命令运行，不依赖项目专用宿主执行回执。
脚本不安装环境、不运行真实 GitHub spike，也不生成签名或模拟产品通过。

测试 API 默认 3082（可用 `--api-port` 指定），浏览器 4300；不复用已有服务。
`E2E_API_PORT` 仅调整 E2E 代理，普通前端代理仍为 3081。测试 API 显式移除真实 worker
配置，在 TEST_DATABASE_URL 对应的可丢弃库中新建独立 schema（名称写入结果，保留供诊断）。
启动的进程在退出时停止。勿运行固定占用 3081 的旧宿主
E2E wrapper 来测试同时运行中的真实 A01 部署。

## A01–A12 → 已有实现和测试

下表是证据索引，列出测试不等于宣布真实边界已通过。当前执行结果见 GH-24 记录。

| 场景 | 实现 | 确定性/本地测试 | 真实边界 |
|---|---|---|---|
| A01 | business、initial/preparation、runtime_service、validation_worker、delivery_service；需求表单 | `requirements.rs`、`delivery.rs::initial_ready_plan_is_durable_and_admission_binds_the_same_worktree`、`validation_store.rs::worker_drives_fixed_candidate_and_one_repair_to_new_validation`；`tools/tests/a01-observe.test.mjs` | `tools/a01-smoke.mjs`；Requirement 1/revision 1 已 Submitted；真实 PR #4，候选 a8e7411 与独立验证绑定 |
| A02 | runtime/protocol/store、runtime_question、runtime_resume | `runtime.rs::persistence_and_full_client_acceptance`、`frames_and_validation_reject_invalid_input`、`runtime_real.rs`；慢于 idle poll 的真实数据库写入不丢 RPC | runtime_real 使用真实锁定 Codex 与脚本化 provider，仅证明协议/进程边界；真实模型计入 A01 |
| A03 | execution/process、run_store、workspace/git_broker | `execution.rs`、`workspaces.rs`、`startup.rs` | 原组/身份屏障、声明前工作保全复用原测试 |
| A04 | validation_store/service、runtime_resume | `validation_store.rs::real_validation_service_success_failure_and_recovery`、`reconciles_durable_process_result_and_rejects_corrupt_ledger`、`runtime.rs::storage_recheck_restores_paid_work_once_without_a_question_or_pause` | 从验证阶段恢复，不重新编码；真实 A01 验证日志及摘要见 GH-24 记录 |
| A05 | delivery_store/worker/control、github_observe | `delivery.rs` 的响应丢失、重复 outbox、取消/合并竞态；`github.rs::generated_pr_branch_requires_explicit_same_repository_source_binding` | 真实 App 预检单列；历史探针 PR 不算产品交付 |
| A06 | validation/runner/store、trusted identity | `validation.rs`、`validation_runner.rs`、`validation_store.rs::durable_exact_evidence_and_read_only_api` | 源码/入口/SHA/证据身份；正式双 Gate 只由独立服务判断 |
| A07 | preparation/service、storage/service | `preparation.rs`、`runtime_environment.rs`、`storage.rs` | 本次普通数据库/浏览器/Runtime 预检与操作员 runtime-preflight；无模型调用的依赖失败 |
| A08 | validation_repair、budget/store | `budget.rs`、`validation_store.rs::repairs_are_atomic_and_requirement_owned`、`runtime.rs` | 跨 Run 不刷新调用/用量；基础设施重试独立于一次代码修复 |
| A09 | execution_control、operator_control、delivery_control | `execution.rs`、`operators.rs`、`delivery.rs::fresh_exact_merge_releases_without_done_and_api_exposes_delivery` | 暂停回答、CI 等待及取消收尾保留队列占用 |
| A10 | storage_lifecycle/store/files/archive/cleanup/consumers/db | `storage.rs::persisted_retention_archive_retries_and_cumulative_admission`：受控时钟、多需求多重试、分类实际字节/数据库占用、保护原件、中断归档/删除、容量拒绝；新增 active Runtime socket 集成测试 | 原需求已恢复并交付；覆盖先意图后快照、未派发恢复跨重启、本地提交未声明完成；开发宿主 PR #40 不充当 Rust A10 证据 |
| A11 | trusted preparation、Runtime transport、外部 App/Gate 服务 | `runtime_environment.rs`、`runtime_real.rs`；本入口前置探针 | 操作员提供 Runtime/GitHub 版本与权限预检；开发子环境无产品配置是预期边界 |
| A12 | operations/inbox、operator API、Angular 四页 | `operators.rs`；`web/angular/e2e` 的 Desktop Chrome/Pixel 7、axe、焦点、错误关联、断线/关闭页面 | A01 浏览器已创建并关闭；集成截图另行复核，业务 Submitted 不等于 Done |

Rust 测试路径均相对 `apps/server/tests/`。不要求每个故障重演真实模型/GitHub，不把脚本化
provider、HTTP fixture 或缺能力 skipped 记录为真实通过。

## 真实 A01 与恢复

由部署负责人准备已授权单仓、固定基线 bundle、锁定 Runtime、custom validation Plan、
独立持久 PostgreSQL、STORAGE_CONFIG 分类预算及受保护执行目录，先执行真实 runtime/App
预检。配置和服务日志保存在其受信环境，只将非秘密版本、摘要与输出送入验收记录。
不要把开发数据库当成该持久产品库，也不要复制签名/授权服务凭据。

首次创建按 [delivery.md](delivery.md) 的输入文件与浏览器入口执行。已有 Ready 时：

```sh
node tools/a01-smoke.mjs INPUT.json NEW_EVIDENCE_DIRECTORY --resume PREVIOUS_RESULT.json
```

续观只发 GET，核对指定仓库、Requirement/revision、交付身份，保存 operations 和 delivery。
存储阻断立即退出 incomplete；旧结果、截图和预算保留。不要盲目重跑创建、撤回 Ready 或
重置 Run/预算。观察到 PR 时记录 Submitted 及源码 SHA、validation_id、Run、PR、介入与消耗；
随后从产品证据 API/服务原件核验独立验证与受信配置。Submitted 仅表示 PR 已提交，
不宣布 CI 成功、合并或业务 Done。

Actions 源默认保持精确 `branch`。需要匹配平台生成分支时，受信部署显式设置该 Actions
源的 `branch_from_pr: true`，按新配置版本审核；仅绑定观察到的同仓 PR head ref。
App、workflow ID/blob SHA、event、候选 head SHA、suite/job/attempt 校验不变。
预检探针分支不会自动成为以后所有产品 PR 的来源证据；旧配置不会自动放宽。

停止前使用已有暂停/停止意图，等待旧进程组静止及工作保全完成；保留 PostgreSQL、原工作区、
原件与预算台账，不以删除目录释放占用。服务重启须通过原 incarnation/进程恢复屏障。
存储故障先查脱敏 scan 错误及分类占用，再使用当前版本的 `storage_recheck`；只有核对真实容量、
目录身份和静止事实后才允许恢复。仍不满足恢复条件时返回冲突，不记录假成功。
对最新同 revision、静止且请求停止的中断执行 Run，成功重检先记录独立的 storage_resume_requested 意图，
即使快照尚未生成或 guard 已由先前重检解除；已接受完成声明或已进入验证的候选仍交由原阶段恢复；仅本地提交未声明完成的工作可恢复执行。
实际续跑须等待快照保全完成并通过现有恢复检查，再复用原恢复管线；不会伪造用户暂停、重新创建 Requirement 或刷新预算。清理失败有持久重试预算，不删除唯一实现，不自动解除用户暂停。

## 样式与试用记录

[UI 来源规范](ui-design-system.md) 记录 arc-admin 提交
`2faa1ca6c1a2b1a45956540e97e68a01532a40f7`、原文件摘要及 Material 22 适配。
令牌文件已入库，构建不读取本机 arc-admin。
[GH-12](quality/gh12/README.md) 是独立构建/健康页基线，
[GH-13](quality/gh13/README.md) 是需求评审表单，
[GH-22](quality/gh22/README.md) 是四页/六问/消耗与可访问性；
[开发环境 #27](quality/remote-environment/README.md) 单列，不能替代业务验收。
M2 登录与 Cloudflare Access 不在本项范围。

第一条真实 A01 完成后即可有限试用。每次填写以下记录，后续观察 5 条不同难度需求，
样本数不作为本项或后续开发的硬门槛；首条已贯通且有人工恢复，不能写作零介入成功。

| 日期/需求 revision | 复杂度与 AC | 源码/配置摘要、Run/验证/PR | 真实结果及实际阶段 | 介入原因/人工时长 | 调用/token/费用及未知项 | 保全/恢复与待解决问题 |
|---|---|---|---|---|---|---|
| 2026-09-19 / 1 | 最小 marker + 精确字节测试，1 AC | [绑定记录](quality/gh24/real-a01.json)；PR #4 | 独立验证成功，Submitted，未合并 | 8 次产品介入；另有部署/预算授权；人工时长未知 | 5 次模型调用；部分 tokens/费用未知 | 原件/所有调用保留，预算显式增量；后续观察稳定性 |
| 后续样本 | | | | | | |

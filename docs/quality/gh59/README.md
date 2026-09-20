# GH-59 / M1-01 验证映射

范围：确定性 Markdown/JSON 导入、版本化父子 Draft 和来源持久化、最小导入/编辑/查看页面。
实现说明与样例见 [输入契约](../../import-drafts.md)。本项不授权产品执行，也不代表完整 0a、B01 或 V1 通过。

## AC → 实际检查

| AC | 测试/检查 | 保留证据（工作区相对路径） |
|---|---|---|
| AC01 | `drafts.rs::deterministic_parser_and_incomplete_data` 与真实浏览器 JSON/Markdown 导入；比较原文与全部解析字段，展示父 AC 和两个含依赖子项 | `artifacts/gh59/acceptance/browser-tests.log`；`acceptance/browser/drafts-*/{json,markdown}-draft.png` |
| AC02 | `drafts.rs::atomic_import_conflicts_legacy_guards_migration_and_real_restart`：真实 PostgreSQL，HTTP 导入/更新，结束应用进程、重新启动后完整响应相等 | `artifacts/gh59/acceptance/workspace-tests.log`；最终覆盖率运行日志 |
| AC03 | 同一 API/数据库测试覆盖非法 kind、重复 ID、悬空/自引用/循环依赖、未登记仓库、过期版本；历史表故障注入验证当前行与历史整体回滚；正确版本成功 | `artifacts/gh59/drafts-tests.log`；最终覆盖率运行日志 |
| AC04 | 新 API 测试比较 Run、预算、全局 owner；父/子草稿身份通过旧 Ready/控制 API 得到 409；现有独立单需求/多仓/Runtime 回归 | `artifacts/gh59/acceptance/workspace-tests.log`、`acceptance/browser-tests.log` |
| AC05 | 独立 schema 先应用 0001–0016 并写旧单需求，再迁移到 0017，旧记录完整 JSON 不变 | `apps/server/tests/drafts.rs`；数据库测试真实日志 |
| AC06 | 桌面 Chrome 与 Pixel 7：键盘提交、全部 AXE 规则检查、无横向溢出、错误可见、重载后编辑；截图人工检查 | `artifacts/gh59/acceptance/browser/` |
| AC07 | Rust fmt / workspace tests / Clippy，Angular lint / unit / build，浏览器与本地 Gate config/secrets/audit；源码风险测量，精确提交双 Gate 由独立宿主执行 | `artifacts/gh59/acceptance/result.json`、`typescript/`；正式双 Gate 待 CI |
| AC08 | 输入格式、版本冲突、迁移、身份作用域、授权边界和未实现项 | `docs/import-drafts.md` 与本文 |

## 本地真实结果

`python3 /opt/symphony-env/run.py python3 tools/single-repo-check.py artifacts/gh59/acceptance --api-port 43821`：PASS。
包含 Rust 工作区 104 个通过记录（含子进程测试输出）、23 个前端单元测试、18 个桌面/手机浏览器测试，以及 fmt、Clippy、build、lint、配置、秘密扫描、架构检查。
Rust `runtime_real` 使用真实锁定 app-server 和本地脚本化 provider，通过且没有外部模型调用。

本地 HTTP 采集：65 个业务 operation/status 变体实际响应通过 Schema 校验；现有接口变化数为 0，客户端无漂移，生成类型匹配。
健康 503 的正式采集仍由既有独立宿主负责，本地业务采集不冒充其结果。

前端原始源码覆盖率探针：新增 18 个函数全部函数/行覆盖，最高 CRAP=5。探针使用固定 collector 的自带依赖；未修改项目锁文件或门禁要求。
Rust 最终工作区覆盖率运行通过，固定源码 collector 测量 991 个函数无阈值违规；新增 32 个函数最高 CRAP=10，最低行覆盖率 20/21（95.2%），最低 region 覆盖率 13/16（81.25%）。证据为 `artifacts/gh59/backend-coverage.log`、`backend-llvm.json`、`backend-risk.json` 和 `backend-risk-summary.json`。没有本地签名或正式双 Gate 通过声明。

## 证据与边界

工作区 `.symphony-evidence.json` 使用 `symphony-evidence/v1`，逐项列出真实日志、原始采集、测量及截图路径和 SHA-256。
这些工作区产物不作为产品源码发布。after_run 归档及 before_remove 复验属于宿主职责；Agent 未取得归档回执，不声称宿主已完成复制。

预检曾发现提供的 E2E helper 固定端口 3081 被占用；改用独立端口与独立 schema 后验收通过。
直接在前后测试复用的数据上运行曾触发存储保护，保留失败日志；正式本地验证使用新 schema 或重建的一次性测试夹具。
旧生产服务、需求、预算、占用者、模型账本及受保护策略未修改。

仍未实现整组 AC 覆盖映射/Ready、依赖领取、validation_only 执行、模型生成和自动合并。A13 的四份历史原件缺失仍保持原记录。

# GH-63 / M1-04 本地验证记录

基线 `a7636b68205c30a219a837ca42ecc537d94ed60f`（GH-62 / PR #67）。已通过 GitHub API 确认前置 PR #60、#66、#67 均合并；原始身份摘要在 `artifacts/gh63/prerequisites.json`。仅修改 assigned `symphony/GH-63` 工作区，未变更生产服务、占用者、预算、凭据、CI、可信门禁策略或摘要。

## 实际命令与结果

| 检查 | 结果与证据 |
|---|---|
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | PASS；含真实 PostgreSQL、5 条新组队列协议测试、真实锁定 Runtime，以及旧执行/取消/预算/存储/多仓回归；`artifacts/gh63/backend-tests.log` |
| `cargo fmt --all -- --check` | PASS；`artifacts/gh63/format.log` |
| `python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS；`artifacts/gh63/clippy.log` |
| `npm --prefix web/angular run lint` | PASS；`artifacts/gh63/frontend-lint.log` |
| `npm --prefix web/angular run test -- --watch=false` | PASS：10 文件 / 30 测试；`artifacts/gh63/frontend-tests.log` |
| `npm --prefix web/angular run build` | PASS；`artifacts/gh63/frontend-build.log` |
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py` | PASS：22 测试，Desktop Chrome / Pixel 7；独占临时回环 API，无外部写入；`artifacts/gh63/e2e.log`、`e2e-api.log`、`browser/` |
| `python3 tools/gate.py config check` | PASS；`artifacts/gh63/gate-config.log` |
| 本地 `verify --profile hook --all` 诊断 | 秘密扫描、架构、format、Clippy、lint PASS；质量汇总明确 BLOCKED：普通工作区没有宿主的 `.harness-gate/runtime/hook-state.json`。不是正式 Gate PASS；`artifacts/gh63/gate-hook-all.log` |

首次 staged hook 的快照没有前端依赖，lint 报 `ng: not found`；随后 all-scope 诊断的 lint 通过。没有复制签名/受信输入或修改门禁来绕过 blocked。精确发布提交的覆盖率、CRAP ≤10、Harness-Gate 与 Trusted Harness-Gate 仍由独立宿主实际测量和判定；此记录不代替它们。

现有完整 Rust 套件留下合成存储策略（指向测试结束已删除的临时目录），浏览器套件也留下已授权合成组记录。因此套件间使用供给的 `dbctl.py recreate test` 重建 disposable 测试库。最初复用数据库时，存储保护阻止取消收尾，遗留组在旧单需求路由回归中仍排在队首；重建后完整套件通过。失败日志、保存的合成存储策略和实际重建回执均保留在 `artifacts/gh63/`，没有删除失败证据、修改旧断言、解除生产锁或重建 persistent dev fixture。

## AC 与范围

完整 [AC→测试映射和完成事实接口](../../group-queue.md#验收证据映射)。新测试覆盖跨组/跨仓/旧需求唯一 owner、事务故障回滚、重复领取、同仓祖先核验与跨仓版本/产物绑定、来源/版本/验收计划拒绝、暂停新 incarnation、累计/预留/迟到预算、旧评审和预算入口不可绕过、父项非 Done、validation_only 不创建编码 Run。

浏览器证据含组顺序/依赖、未实现能力、桌面/手机截图、键盘确认、axe、无页面横向溢出、减少动效及强制颜色。已人工查看 desktop/mobile `review-top.png`：标题、反馈、队列卡片及长文换行可读，无遮挡；沿用现有语义颜色/间距，axe 无对比度违规。

`FixtureVerifier` 和协议合并数据都是固定合成夹具，仅验证接口，不代表 M3 的线上全链路验收。产品未配置线上完成事实生产者；缺适用验收、validation_only 与父项最终业务验收明确等待。M1-05 的受控组变更仍不在本 PR。历史 A13 缺失原件按[原恢复索引](../gh25/evidence-recovery.md)如实保留。

工作区根 `.symphony-evidence.json` 使用 `symphony-evidence/v1`，逐文件保存实际 SHA-256；宿主在 after_run 后归档。Agent 不宣称已取得归档回执。产物不作为产品线上完成事实，也不作为受信签名。

# GH-65 / M1-06 集成验收记录

开发基线 `08bf1f4d10ff90e09e8fdba52641e1478e125228` 包含已合并的 PR #60、#66、#67、#68、#69。
`artifacts/gh65/dependencies.json` 保存 GitHub 实际返回的合并身份。
本次仅修改集成测试和文档，不部署生产，不修改产品业务事实、凭据、门禁政策或可信摘要。

[使用、升级/恢复及 Issue→AC→测试矩阵](../../m1-delivery.md)给出完整操作路径。
本目录的 `evidence.json` 将实际原件路径对应到 SHA-256；工作区 `.symphony-evidence.json`
使用 `symphony-evidence/v1` 供宿主归档。Agent 不宣称已取得 after_run 归档回执。
`artifacts/gh65/validated-source.json` 绑定实际测试的产品、API、迁移及测试源码文件。
发布后的实际远端提交/树身份另存 `artifacts/gh65/publication.json`；不能把本地 baseline HEAD 当成 PR head。

## AC 与原件

| AC | 实际测试及保留路径（均在 artifacts/gh65/ 下） |
|---|---|
| AC01 | `browser-first/` 的完整 24 项真实 API 桌面/手机流程包括 Markdown/JSON 导入、小需求及大需求生成。最终 `browser-final/` 的 `real-generation.json`、`real-generated-document.json`、`real-saved-draft.json`、`m1-authorized.json`、`m1-desktop-readback.json` 保存真实模型→修改→覆盖评审→一次成功授权→跨设备同队列读回；相应截图为真实浏览器输出 |
| AC02 | `cargo-test-complete.log` 的 group_queue/group_edits/execution/runtime/workspaces：实际数据库、产品领取逻辑、本地 Git/进程、唯一 owner、精确基线、重启恢复和编辑/领取两种先获锁次序。外部合并与适用验收 Fact 使用命名测试夹具，不是实际 M3 自动合并或最终业务验收 |
| AC03 | 同一 Cargo 日志中的 group_review/group_edits/generation/drafts/budget/requirements：覆盖/依赖/版本/故障事务无部分 Ready，生成失败/超时有界且草稿安全，编辑保持用量/预留与运行快照，旧单需求回归；默认生成协议 fixture 不冒充真实模型 |
| AC04 | group_queue 中 Submitted/CI/合并观察与适用验收分开检查，unsupported validation_only 不创建 Run；真实 `m1-authorized.json` 为 business_complete=false、completed=0，最后 validation_only 的 requirement_id=null；队列截图显示明确能力等待 |
| AC05 | `browser-first/`、`browser-final/`：键盘、axe、手机/桌面、无横向溢出；`cargo-test-complete.log` 新增 GH-59/0017 数据升级测试及已有实际 API 两次启动测试；`restart/` 保存同一真实生成授权队列在两次新 API 生命周期中的完整读回。格式、Clippy、前端检查见下面命令表；精确 PR 双 Gate 待独立宿主 |
| AC06 | 本索引、使用/恢复说明、逐文件 `evidence.json` 与根 evidence manifest；明确 M2/M3、模拟边界及历史 A13 四份缺失原件，未声称 B01–B08/V1 整体通过 |

## 命令与结果

以下本地检查均通过。最终真实浏览器 2 项通过，随后两次实际 API 重启读回一致。
最终小需求输入/输出 6830/217 token，大需求 6875/876 token（缓存输入 4352 已包含在输入内）；
实际记录完整，耗时分别 12/31 秒。已查看桌面评审与手机队列截图，控件和等待说明可读。

本地检查不是正式签名 Gate。精确 PR head 必须由宿主取得 Harness-Gate 和 Trusted Harness-Gate，
不复用前置 PR 的签名或测量，不把本地配置检查视为双 Gate 已通过。

| 检查 | 日志 |
|---|---|
| `python3 tools/gate.py config check` | `gate-config.log` |
| `cargo fmt --all -- --check` | `fmt.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | `clippy.log` |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked --no-fail-fast` | `cargo-test-complete.log`，含真实 PostgreSQL 与 pinned Runtime |
| `npm run lint` | `frontend-lint-delivery.log` |
| `npm test -- --watch=false` | `frontend-tests.log`，33 项 |
| `npm run build` | `frontend-build.log` |
| 提供的独立 API e2e helper + `GH61_REAL_GENERATION=1` | `browser.log`，24 项；真实模型与 UI fixture 明确分开 |
| 独立 schema、单 worker 的最终真实生成流程 | `real-browser-final.log`；保留的 `real-browser-runner.py` 调用原 package script，只限 generation spec |
| `python3 /opt/symphony-env/run.py python3 artifacts/gh65/restart-readback.py` | `restart-readback.log`、`restart/result.json` |
| `python3 tools/gate.py secrets --json` / `audit --json` | `secrets-final.json`、`audit-final.json`，无发现 |

模型模式需要操作员提供的 `DRAFT_GENERATION_CONFIG` 指向已登录的 Runtime home，本次临时配置只包含路径、模型和时间上限。
没有读取或复制认证文件内容。真实模型采用锁定 0.154.0，实际 thread/turn、user-agent、用量和结果保存在生成记录。
最终浏览器 schema 为 `gh65_browser_final`；未与后端回归同时争用产品单实例锁。

## 保留的失败与修正

- `cargo-test.log`：独立 Runtime 场景虽然使用不同 schema，仍争用数据库级 advisory lock 13002，出现锁超时和测试进程输出缺失。将四个独立数据库场景串行隔离，各场景内部竞态与生产锁超时不变。
- `cargo-test-final.log`：浏览器留下的已授权队列影响旧多仓 fixture 的领取预期。保全浏览器原件后重建提供的一次性 test 数据库；最终浏览器改用独立 schema。
- `cargo-test-clean.log`：一次执行控制启动等待未看到 launch.json；带 backtrace 的 `execution-diagnostic.log` 单独复跑通过。未通过增加产品超时或额度掩盖失败，原件与最终完整运行并存。
- 首轮大需求键盘父 AC 被测试脚本映射到集成项的通用 AC。检查真实生成文档后，将键盘条件映射到专门的键盘 AC/步骤并重跑，首轮截图/记录仍保留，不作为最终语义覆盖证据。

仅记录 B01、B02、B07 的上述子场景，不宣布完整 B01/B02/B07、B01–B08、V1 或 Phase 0a 发布通过。
M2 登录/公网部署/通知、M3 自动合并/关联修复/validation_only 执行及父级最终业务验收仍待后续。
A13 四份历史原件仍 unresolved，详见[既有恢复记录](../gh25/evidence-recovery.md)，不补造旧证据。

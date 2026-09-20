# GH-64 / M1-05 验证记录

从包含 PR #68 的 main `73f1c331a7b870e7cacc28e02610b6642203a152` 开发；通过 GitHub API 确认 #60/#66/#67/#68 均已合并。工作区保留实际预检、失败诊断及最终结果，未修改门禁、可信摘要、预算策略或生产配置。

## 实际检查

| 检查 | 结果与证据 |
| --- | --- |
| PostgreSQL 16 fixture 及基线 | 实际 SQL 查询通过；原 124 项完整 Rust 测试（含真实 Runtime）通过。`artifacts/gh64/preflight/` |
| `cargo fmt --all -- --check` | PASS；`fmt-delivery.log` |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | PASS：135 项；`cargo-test-delivery.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS；`clippy-delivery.log` |
| `cargo build --workspace --locked` | PASS；`backend-build-delivery.log` |
| `npm --prefix web/angular run lint` | PASS；`frontend-lint-final.log` |
| `npm --prefix web/angular run test -- --watch=false` | PASS：11 文件 / 33 测试；`frontend-tests-final.log` |
| `npm --prefix web/angular run build` | PASS；`frontend-build-final.log` |
| fixture wrapper 中的 `/opt/symphony-env/e2e.py` | PASS：24 项 Desktop Chrome / Pixel 7；`e2e-3.log`、`e2e-api-final.log`、`browser-final/` |
| 真实 API 契约与进程重启 | PASS：85 个响应、24 个客户端调用无类型漂移；独占临时端口、独立合成 schema。保存实际重排、暂停与待评审差异，终止 API 进程再启动，顺序、版本、预算、授权、暂停及待评审方案保持；`api-capture-delivery.log`、`api-check-delivery.log`、`api/restart.json` |
| `python3 tools/gate.py config check` | PASS；`gate-config-delivery.log` |
| 完整 `cargo llvm-cov --workspace --locked --json` 与锁定 Rust source collector | PASS：135 项测试；1,210 个函数全部映射，逐函数覆盖率均 ≥80%、CRAP ≤10、无缺失。行 12,007/12,231；region 21,182/23,072。`coverage-delivery.json`、`source-measurement-delivery.json`、`measurement-summary-delivery.json` |
| 锁定 TypeScript collector 本地诊断 | PASS：21 个源文件，逐函数覆盖率/CRAP 检查无失败或缺失；`typescript-risk-final.log`、`frontend-quality-final/` |

本表中未写全的证据路径均位于 `artifacts/gh64/`。API 本地采集不停止测试数据库来制造健康 503；正式完整健康响应由宿主采集。该限制不替代已有健康失败单测/浏览器回归，也不把本地部分采集称为完整签名门禁。

## AC、回归与边界

完整 [AC→测试映射](../../queue-editing.md#ac-与验证) 包含：合法重排及非法依赖/重复/过期拒绝；真实数据库共享锁的两种领取竞态；闭包冻结、保留未影响授权/完成事实；缺父 AC 覆盖与政策漂移拒绝；故障注入无部分授权；增删、稳定身份、预算已用/预留；暂停后重启与取消依赖语义。

旧 Draft 编辑/生成发布与首次队列投影也共用事务锁，实际数据库测试覆盖双方先获锁的顺序。新增测试还覆盖同仓前序基线的隐式依赖、混合内容变更与重排、已撤回授权的队列不得部分投影、kind 重绑定拒绝、新 validation_only 不生成 Requirement/Run，以及原授权内容与变化版本分别保留。
已人工查看 desktop/mobile `queue-reordered.png`：重排控件、顺序与待办信息在页面内，手机纵向布局无横向溢出；浏览器断言键盘、axe、减少动效和强制颜色。

已领取或已有不可变 initial Run 输入的项拒绝此未开始入口。已有停止、保全和按授权版本恢复控制通过完整回归；没有新增运行中输入替换捷径。共享范围的变更不能重写已完成/运行项。仓库/kind 的稳定身份不可改绑，移除及重加入不清除原账本。
完成事实使用明确标记的协议/持久化夹具，不代表 M3 真实业务验收。validation_only 执行、自动合并、父级最终业务验收仍未实现。历史 A13 缺失原件按[原索引](../gh25/evidence-recovery.md)保留，未补造。

## 保留的失败与修正

- 首次移动并发测试的临时数组借用未通过编译，改为拥有生命周期的异步任务。
- 完成持久化夹具最初只含 source 字段，真实依赖解码拒绝；修正为完整、明确标记为合成的 Fact 结构。
- 合并后的手机长流程触及原 30 秒超时，拆成原授权与队列编辑两项测试，未扩大超时。
- 手机并发测试发现无关仓库登记会造成策略冲突；快照比较已限定为本组使用的仓库。
- 现有 TypeScript 插桩器不支持 Angular signal input 初始化；改为局部注入共享父评审上下文。箭头对象字面量改为显式返回块，以保持准确源码锚点；未修改采集器。
- Rust 引用字段闭包缺 native mapping，改为等价字符串切片。将事务步骤拆为小函数并补混合编辑重排测试，达到原覆盖率/CRAP 门槛。
- API 重启辅助脚本第一次选中了 PUT 评审返回的未授权视图，现以 GET 当前已授权视图为准，重启检查通过。

各失败日志与最终日志并存。普通数据库测试和浏览器测试间仅重建提供的一次性 test fixture，未操作 persistent dev 或生产数据库。

工作区根 `.symphony-evidence.json` 使用 `symphony-evidence/v1`，逐文件声明实际 SHA-256，供宿主 after_run 归档。Agent 不宣称已取得宿主归档回执。本地测量没有签名；精确发布提交仍等待 Harness-Gate 与 Trusted Harness-Gate，不能据此宣布产品整体完成。

## PR #69 第一次 CI 修复

精确提交 `23b6cf69e78c9d1da9c44e63aee3abedb549ac6e` 的 Actions
`35521631782/1` 未通过：前端测试桩的空 async 方法违反 lint，HTTP
collector 报告 adapter exit 1。上表的本地 lint 记录不证明该提交通过。
宿主诊断恢复后保存在 `artifacts/gh64/ci-repair-1/host/diagnostic.json`；
此前缺少诊断的阻塞记录仍保留。

测试桩改为返回 resolved Promise。另以锁定 HTTP collector 的来源检查复现
`unrecognized generated-client origin`：生成文件携带的旧摘要既不等于
当前契约，也不等于基线。修正为当前 OpenAPI 文件的实际 SHA-256；类型定义、
业务行为、门禁策略和可信基线均未改变。宿主未提供 adapter 的详细 stderr，
此处记录的是本地确定复现，不冒称已读取宿主内部异常。

修复后重新执行 lint、33 项前端测试、生产 build、Gate config check 及
85 个已保留真实业务响应/24 个客户端调用的契约校验，全部通过。
原始日志与来源摘要检查位于 `artifacts/gh64/ci-repair-1/`。
此次未修改 Rust 或运行时行为，沿用前述数据库、Runtime、桌面/手机证据；
未重跑浏览器或声称本地完整签名质量门禁通过。新提交仍须宿主精确 SHA 双 Gate。

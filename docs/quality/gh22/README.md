# GH-22 开发验证

本记录对应保留工作区的 GH-22 实现，不是精确发布提交的签名 Gate 结果。前置 GH-21 的 PR #48 已合并，published head `de1cbd2aa02af9d2d714f000fb88a4e2cd84261f` 的适用保护检查已通过；当前控制器基线为 PR #49 提供 HTTP 夹具能力后的 `77dd3776c5515b805d473ab1c1525adc03a80bdc`。

## 本次修正

- 复用原实现，新增取消收尾完成后移出待办箱的筛选；历史暂停/旧问题不会制造永久待办。
- 验证跨 Run 用量求和、未知值保留、人工等待累计和阶段记录；静止 Run 不启动新的阶段计时。
- 重新检查保留预检尝试历史，不增加代码修复次数；控制幂等与当前版本检查共用现有事务锁。
- 补齐 HTTP 404/409/422 负向契约和真实请求场景；问题重复/陈旧回答与非所属证据访问被拒绝。
- 四页复用公共 UI token，详情/待办读取持久化事实，页面断线保留陈旧提示，按钮避免重复提交。

## 验证记录

- `python3 tools/gate.py config check`：配置与既有门禁要求有效。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `RUST_TEST_THREADS=1 python3 /opt/symphony-env/run.py cargo test --workspace --locked`：通过，包含真实 `runtime_real`、控制/回答/取消、持久化指标及零介入率用例。
- `npm run lint`、`npm run build`、`npm test -- --watch=false`（在 `web/angular`）：通过；7 个测试文件、18 个测试。
- TypeScript 原源码插桩及安装的 rc.4 collector：64 个函数均满足逐函数行覆盖率/函数覆盖率 ≥80% 和 CRAP ≤10；最低函数行覆盖率 81.818%，最高 CRAP 7.294516。所有含运行代码文件的行覆盖率也达标。

Rust 安装的 rc.3 原生 collector 最终测量 718 个函数：无缺失或不支持的指标，逐函数最低行/region 覆盖率均为 80%，最高 CRAP 为 10。完整结果与源码/原生计数绑定，保留在 `target/gh22-quality-complete/measurements.json` 和 `summary.json`。

- `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py`：最终 Desktop Chrome / Pixel 7 共 14 个测试通过，覆盖四页、六问、暂停后回答、重复/陈旧回答、断线接续及关闭页面后的后台取消收尾。四页 axe/WCAG A/AA、表单错误关联、键盘操作与无横向溢出检查通过；详情页桌面/手机截图已人工查看，无裁切或遮挡。
- 干净一次性数据库加载 `api/capture-fixture.sql`，用实际 API 执行全部 `api/capture-scenarios.json`；额外通过关闭本地数据库 TCP 转发制造真实 health 503：共 39 条真实 HTTP 响应。安装的 rc.4 collector 与仓库 `api/baseline.json` 比较得到 breaking_changes=0、client_drift=false、compatible=true。原始响应和输入/二进制 SHA-256 在 `target/gh22-http-final/`；复现脚本保留于 `.agent-tmp/gh22-http-check.py`，命令为 `python3 /opt/symphony-env/run.py python3 .agent-tmp/gh22-http-check.py`。正式宿主仍独立采集并使用它持有的批准基线。

历史 `blocked.md` 和 `http-fixture-recovery.md` 保留先前环境失败及恢复，不作为当前源码通过证明。

原始日志保留在 `.agent-tmp/gh22-*-complete.log` 和 `gh22-*-accepted.log`。Rust 原生覆盖数据、源码清单、二进制和 profile 保留在 `target/gh22-quality-complete/`；TypeScript 最终采集在 `.agent-tmp/codexsymphony-ts-risk-UsniEh/`。它们都是实际执行采集，未使用另一版本的测量结果替代。

本轮早期并行测试在高编译负载下触发共享数据库 advisory lock 的 500ms 超时。最终本地 Cargo/覆盖率使用 `RUST_TEST_THREADS=1`，保留测试内部的显式并发场景，未修改产品锁超时、测试断言或 CI 配置。未串行的独立宿主 CI 仍需自行验证。源码采集曾拒绝两个表达式闭包，已改成等价的具名函数/分支；原始采集随后揭示两个 CRAP 超限函数，按读取与事务职责拆分后重新测量。

## 边界

#23 尚未提供完整材料生命周期与容量统计。界面显示未接入，不编造压缩、过期、存储预算数值，也不把材料缺失改写成历史验证失败。业务浏览器夹具是 SQL 持久化状态与真实 HTTP，不冒充真实模型到 GitHub 的 A01。真实 Runtime 测试单独执行。

本次不执行真实 GitHub/cloud/通知副作用 spike；不合并或关闭 Issue。最终发布提交仍必须由独立验收服务获取并通过双 Gate，开发工作区无签名密钥。

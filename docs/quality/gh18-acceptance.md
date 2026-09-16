# GH-18 本机验收记录

2026-09-16，分支 `symphony/GH-18`，基线
`893d34f86d79feab4720e580284903a7046b27de`（#17 / PR #38 合并）。
已通过宿主 GitHub API 核对前置 PR 的合并状态，其精确 head
`9b531e448cb69ca06f0953d5b165ac861c455d67` 上的 `Harness-Gate` 和
`Trusted Harness-Gate` 均为 success。工作区起始无未提交实现。

## 实际准备

- 宿主 `github_api` 可读取仓库、Issue #18、前置 PR 与检查结果。
- `python3 /opt/symphony-env/execution_readiness.py` 成功；实际 Agent
  workspace-write 命令沙箱、UID、固定工具、可写 target、只读 Git、隐藏宿主
  凭据及有效网络配置均就绪，零模型调用。规范回执为
  `/opt/symphony-env/execution-readiness.json`，sample ID
  `17087bf2819e4ab89cfc902352d6ffae`。
- 经 `/opt/symphony-env/run.py` 对 TEST_DATABASE_URL、DEV_DATABASE_URL
  分别执行 `psql ... -X -v ON_ERROR_STOP=1 -Atc 'SELECT 1'`，均返回 1。
- `web/angular` 中 `npm ci --offline --no-audit --no-fund` 成功。
  未更改依赖锁、网络白名单或宿主权限。

## 覆盖场景

真实 PostgreSQL 事务测试覆盖：并发预留只允许余额内的请求、重复调用意图及
不同参数冲突、重复／乱序累计事件、事件 ID 内容冲突、数据库重新连接后的
未知调用、先完成信号后用量、实际结算替换预留、旧 Run 延迟结算、跨 Run／
修订共享消费、观测超额后停止、增量授权幂等及版本冲突、历史授权保留、
未知／缓存用量下界、算术溢出拒绝执行、无模型配置拒绝调用、人工等待不扣
模型工作时间及八小时绝对寿命。现有 API 测试证明重新评审更高策略限额
仍保留原账户；准备测试证明领取的 Run 保存评审时的模型。

准备重试沿用原事务记录：30/120 秒、两次自动重试、期限与未知探测守卫，
并补充错误类别的自动重试限制和历史中的额度事实。显式用户恢复保留总次数
及历史，不创建未启用阶段记录，不改变代码修复权限或启动模型。

## 最终本机结果

- `cargo fmt --all -- --check`：exit 0。
- `CARGO_TARGET_DIR="$PWD/target" cargo clippy --workspace --all-targets --locked -- -D warnings`：exit 0；`target/gh18-clippy.log`。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo test --workspace --locked`：exit 0，41 项通过；`target/gh18-tests-final.log`。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo llvm-cov --workspace --locked --json --output-path target/gh18-coverage.json`：最终完整运行 exit 0，41 项通过；`target/gh18-coverage.log`。
- `python3 target/gh18-measure.py`：exit 0，407 个生产源码函数，零违规；最大 CRAP 10，最低函数行／region 覆盖率均为 4/5。原始 LLVM、`target/gh18-source-measurement.json`、`target/gh18-source-measurement-summary.log` 及 `target/gh18-measurement-totals.json` 留存在工作区；重新核对源码 SHA256 一致。
- `web/angular` 中 `npm run lint`、`npm test -- --watch=false`（11 项）、`npm run build`：均 exit 0。通过固定 rc.4 合约 CLI 重新生成并格式化可选模型字段的类型。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" EXECUTION_DIRECTORY="$PWD/target/gh18-contract-execution" python3 tools/check-requirement-contract.py`：exit 0，真实 API 采集 25 个声明的 operation/status 变体；`target/gh18-contract.log` 及 `target/gh13-contract-observations.json`。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" EXECUTION_DIRECTORY="$PWD/target/gh18-e2e-execution" python3 /opt/symphony-env/e2e.py`：exit 0，桌面／移动端 10 项通过；`target/gh18-e2e.log`。
- `python3 tools/gate.py config check`、`git diff --check`：exit 0。

源码测量使用已固定的 rust-source-risk rc.2；归档与已安装文件逐字节一致，
归档 SHA256 为 `aabdbbafa78b20afa3e500b58e74a132002996838b9d812e4ea15f1b193f69b8`。
采集器不支持的内联错误构造位置已通过普通命名转换函数解决，未修改采集器、
测量系列、阈值、requiredness、受信输入或批准配置。

早期运行曾在原有后代恢复与服务关闭的五秒断言超时，已保存
`target/gh18-coverage-recovery-timeout.log`、`target/gh18-coverage-live-timeout.log`
及 `target/gh18-coverage-shutdown-timeout.log`；定点复现与最终完整测试／覆盖率
运行均通过。只为恢复断言增加 incarnation 诊断，没有修改时限。最终测量来自
随后完整成功的同一源码运行，不使用之前失败运行拼接出的报告。

## PR #39 CI 修复（attempt 1）

失败 head 为 `e9f98b25acb57d6b6b66d2237e316cb9b9900c16`，Actions
`35044911288/1`，`Harness-Gate` check `104632546155`。已核对宿主诊断中的
source SHA / attempt；后端覆盖率采集在 `execution_acceptance` 的
`after-crash` 恢复等待超时，没有配置身份变更。脱敏诊断保存在
`target/gh18-repair-host-diagnostic.json`。

修复仅调整三个集成测试及本记录：

- 恢复测试先提交停止请求、补齐启动记录，再开始原有五秒收敛等待；首次
  文件 fsync / 数据库准入不再占用这个等待窗口。仍要求匹配身份的真实
  静止回执，且保留写者未启动、队列未释放等断言；超时附带耗时、轮数、
  storage latch、Run 身份和回执诊断。增加六秒 Run 行锁竞争场景，旧 helper
  在同一 `after-crash` 断言失败（exit 101，
  `target/gh18-repair-regression-before.log`），用于复现计时边界误报。
  原 CI 日志没有每步耗时，不能据此认定宿主当时也发生了相同的行锁竞争。
- 准备重试测试以落库的 `next_attempt_at` 驱动后续尝试，不再假设探测
  零耗时。加入一秒慢失败探测，检查完成后退避以及截止前拒绝执行；保留
  三次实际尝试和最终准入断言。早期完整覆盖率运行曾实际得到 2 而非 3
  次尝试，日志为 `target/gh18-repair-before-coverage.log`。
- 一次完整测试通过后，真实覆盖率测量发现 `github_service::poll` 仅覆盖
  2/5 行、6/11 region（`target/gh18-repair-measurement-before.log`）。原测试
  观察到仓库 capability 就中止 worker，该写入尚不是完整轮询结束。
  改为等待两轮实际 PR 观察落库，包括定时等待后的再次刷新，验证结果为
  `Unmerged`，最后等待 worker 取消完成；使用本机 HTTP fixture，无外部写入。

排查中一次执行诊断与完整覆盖率共用了 fixture，出现身份记录冲突；该诊断
（`target/gh18-repair-diagnostic.log`）与重叠的覆盖率报告均不作为验收依据。
最终数据库测试按顺序执行。普通执行及独立覆盖率排查通过记录分别为
`target/gh18-repair-reproduction.log`、`target/gh18-repair-diagnostic-coverage.log`。

生产代码、运行时超时、重试策略、门禁配置及阈值均保持失败 head 原样。
新 head 仍需独立宿主重新采集并通过两个受保护检查。

本次最终验证：

- `cargo fmt --all -- --check`：exit 0。
- `CARGO_TARGET_DIR="$PWD/target" cargo clippy --workspace --all-targets --locked -- -D warnings`：exit 0，`target/gh18-repair-clippy.log`。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo test --workspace --locked`：exit 0，41 项通过，`target/gh18-repair-tests.log`。
- `python3 /opt/symphony-env/run.py env CARGO_TARGET_DIR="$PWD/target" cargo llvm-cov --workspace --locked --json --output-path target/gh18-coverage.json`：exit 0，41 项通过，`target/gh18-repair-coverage.log`。
- `python3 target/gh18-measure.py`：exit 0；407 个函数零违规，最大 CRAP 10，最低行／region 覆盖率均为 4/5；`target/gh18-repair-measurement-summary.log` 和 `target/gh18-source-measurement.json`。
- `python3 tools/gate.py config check`、`git diff --check`：exit 0。

前端、OpenAPI 和生产源码与失败 head 相同，本次未重复执行前端／E2E 检查；
它们此前的本机结果见上文，不替代新 head 的完整宿主门禁。

## 未验证边界

这是 A08 额度基础与 A02 用量基础的验收；没有真实模型调用或金额计算。
Runtime 协议解析须将其累计用量规范化为单 turn 值并接入本调用守卫与活动计时，
真实调度、模型继续、自动代码修复 ordinal、验证／交接／清理阶段和额度增量的
页面控制仍由后续任务接入。模型用途标记本身不授予修复权限。

本机测量无签名，不代表产品完整 0a 或受保护 GitHub 检查已通过。独立宿主必须
针对本 PR 精确发布 SHA 重新采集并执行完整 Gate；交接时 CI 仍待控制器确认。

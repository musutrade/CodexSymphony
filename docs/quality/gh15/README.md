> 历史验收记录：执行边界已于 2026-09-16 改为可信开发环境。本文旧沙箱、白名单和专用验收入口不再是当前要求；以主规格及 docs/trusted-development.md 为准。历史结果保持原样。

# GH-15 本地验收记录

2026-09-15。基线 `712ee98344bf5fb2432f945c240e1c7b725c0135`（#35）。
前置 #14 的 PR #35 已合并；精确 PR head `6f5f922174a1535b7c89ef70b26f11cd43d5fab5`
的 Harness-Gate（App 15368）及 Trusted Harness-Gate（App 4867361）均为 success。
通过宿主 `github_api` 读取核实；没有以关闭 Issue 代替依赖验收。

## 准备与实际检查

| 命令 | 最终结果 |
|---|---|
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/verify.py` | test/dev 查询、容器重建、测试数据丢弃、dev 合成数据保留、只读 Git/来源和隐藏宿主密钥边界通过 |
| `/usr/bin/git init --bare target/gh15/git-readiness/repo.git` | 工作区内 Git fixture 可写；开发仓库 `.git` 仍只读 |
| `cargo fmt --all -- --check` | 通过 |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | 23 项通过，含 7 项工作区主测试与 2 个由其实际启动的子进程测试入口 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过，无警告 |
| `python3 tools/gate.py config check` | 通过，required、阈值与受信配置未改变 |
| `git diff --check` | 通过 |

最终普通测试首轮在已有 `execution_acceptance` 的监督器身份握手处超时；
相同源码的原生覆盖采集已通过，随后完整普通测试重跑也通过。
未修改监督器时限或测试策略。原日志保留为 `target/gh15/tests-first-final-attempt.log`。
新增数据库测试使用独立 schema，因 PostgreSQL advisory lock 是数据库级而在测试文件内串行执行。
迁移新增 Run 的单调序号后，通过供给的 `dbctl.py recreate test` 重建了可丢弃 test fixture；未修改 dev 数据。

## 实际源码采集

```sh
python3 /opt/symphony-env/run.py python3 \
  /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.2/capture.py \
  --repository "$PWD" --output "$PWD/target/gh15/source-capture-final" \
  --target-dir "$PWD/target/gh15/coverage-target" \
  --source-root apps/server/src --input Cargo.toml --input Cargo.lock \
  --input apps --input migrations --manifest apps/server/Cargo.toml
```

固定插件重新导出保留的 LLVM 对象与 profraw，229 个生产 callable 全部有精确原生映射；
最大 CRAP **10/1**，最低逐函数行覆盖率 **4/5**，最低逐函数 region 覆盖率 **4/5**，无缺失或超限。
测量系列保持 `measurement-series/v1:b79abd709904548fbed1042b2e1ceb146cc3fedc75bc55744eb475b84cdd1db9`。
逐个核对 capture input 的 SHA-256 与最终工作区源文件一致。
概要见 [local-summary.json](local-summary.json)，原件在 `target/gh15/source-capture-final/`。

磁盘写失败使用 Linux 文件大小软限制，在真实文件写至 8192 字节时失败。
子进程在断言前恢复测试软限制，让 LLVM 正常刷新真实计数；初次把 profile 一并截断的失败采集仍保留，未用损坏计数生成通过结论。
CRAP 超限函数按保全步骤拆分，表达式闭包改为明确的命名函数以获得精确映射；没有修改 collector 或隐藏源文件。

## 验收映射与边界

- A03 工作 / A04：真实 staged/unstaged diff、未跟踪源码/测试/进度、二进制、删除与执行位恢复一致；可达中间提交在替换 canonical 后仍完整。新 Run 使用保存的 HEAD，不隐式更新到 main。
- 文件写失败、SQL 保存失败、文件完成后的进程退出均留下原件和 partial/pending，无虚假完整引用；重复请求不覆盖原档，新 Run 的顺序不依赖时钟。
- A06 路径/身份部分：hooks 不执行，helper 配置被拒绝；越界链接、特殊文件、错误归属、历史凭证路径及损坏 bundle 被拒绝；Linux openat2 原子拒绝路径中的链接。
- 当前阶段选择验证候选或完整工作，失效/历史候选不能再次交接。没有 AgentRun 成功→PR、验证 PASS→Done 或 FAIL→执行失败的推论。

实现边界见 [工作区与 Git Broker](../../workspaces.md)。没有前端/API 契约修改，未运行真实 GitHub/cloud/通知 spike。
Runtime、强制执行沙箱、预检/预算、独立验证身份和远端交付仍按后续任务接入，Ready 不开始真实编码。
完整宿主质量与两个受保护检查仍须针对发布后的精确 SHA 运行；本记录不是远端 PASS 或产品 0a 整体验收。

## PR #36 CI 修复（attempt 1）

失败 head `3ee16419537b5be25630a6563085d115c8bf361b`、Actions `34974051650/1`。
操作员导出的精确 head 诊断及四份日志摘要核验一致，原件保留在
`target/gh15/host-diagnostics/34974051650-1/`。失败发生于原生覆盖采集的
`execution_acceptance`：监督器身份握手超时。`Run directory required` 来自
缺少参数的负向测试，与失败无关。

临时打开子进程 stderr 并记录持久化步骤用时，使用同一原生测试产物复现：
启动标记完成于 4.213552593 秒，身份解析于 4.514988418 秒，身份写完于
7.956087371 秒；期间旧的约 2 秒窗口已经返回失败。记录在
`target/gh15/repair-reproduction-both/attempt-0.log`。这是本地复现确认的磁盘
持久化延迟；受信宿主原失败没有子进程诊断，不能声称其内部时序已被直接测量。

握手现在最多轮询 750 次、间隔 20ms（原 100 次），给真实持久化约 15 秒的
有界等待窗口。仍须核验身份、提交数据库身份并重新检查授权才能写入启动许可；
超过窗口仍保留 launch intent、请求停止并阻断恢复。没有改变 Gate 阈值。
临时 stderr 和计时探针均已移除。

回归测试用真实 helper 和释放文件控制身份发布：等待 3 秒后，确认身份、启动
许可与工作文件仍不存在，再释放 helper，核对完整启动和静止凭据。
暂停握手场景也延迟 3 秒，验证暂停仍禁止 writer 启动；原缺失身份、身份不符、
数据库锁竞争和旧 Run 场景继续执行。修复后实际验证见
[repair-summary.json](repair-summary.json)。最终原生采集的输入须与发布源码一致；
完整受信宿主和双检查仍交由控制器在新 head 验收。

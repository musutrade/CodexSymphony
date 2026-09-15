# GH-14 本地验证记录（2026-09-15）

基线为 `a6a9ca994abe8311d0b2d5fc18bc4355cce2eba3`，即前置 GH-13 的 [PR #34](https://github.com/musutrade/CodexSymphony/pull/34) 合并提交。GitHub API 已核实 #34 已合并，其 PR head `07c659a90d9bdf09920e57c34fb1f014a375fd2b` 的 Harness-Gate / Trusted Harness-Gate 均成功。工作区保留控制器分支 `symphony/GH-14`；本地 HEAD 不代表后续发布 SHA。

## 实际检查

所有数据库命令通过 `python3 /opt/symphony-env/run.py` 使用供给的 disposable PostgreSQL 16；Rust 构建使用工作区内 `CARGO_TARGET_DIR="$PWD/target"`。原始日志留在该工作区 `target/gh14/`，摘要与源文件摘要见 [local-summary.json](local-summary.json)。摘要是本次真实采集的未签名本地记录，不是可复用的受信 Gate 输入。

| 命令 | 结果 |
|---|---|
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/verify.py` | test/dev 查询、容器重建身份、测试数据丢弃、dev 合成数据保留、只读 Git/来源和敏感文件不可见均通过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过，无警告 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | 14 项通过；其中执行控制集成测试包含多个有状态故障场景 |
| `npm ci --offline --no-audit --no-fund`（web/angular） | 通过 |
| `npm run lint` / `npm run build`（web/angular） | 通过 |
| `npm test -- --watch=false`（web/angular） | 最终 5 个测试文件、11 项通过 |
| `python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py` | 真实 API、桌面/手机浏览器共 10 项通过 |
| `python3 /opt/symphony-env/run.py python3 tools/check-requirement-contract.py` | 25 个去重的真实接口/状态观察；重复暂停也执行，但不重复提交同一状态的 collector 观察 |
| `node target/gh14/check-http.cjs` | 用固定 HTTP collector 的 schema/type 验证器核对全部本地观察及生成类型，通过；健康 503 明确留给宿主采集 |
| `python3 tools/gate.py config check` | 通过，配置和 required 阈值保持原值 |
| `git diff --check` | 通过 |

首次并行运行前端构建/测试时，一个已有启动测试触及原 5s 超时；单独复跑以及最终生成类型后的检查均通过，没有改超时或测试策略。主机锁加入后，原服务启动测试之间的并行抢占已通过测试内互斥解决；真正的第二服务实例仍必须被拒绝。

## 源码质量采集

实际使用已固定的 `rust-source-risk 0.1.0-rc.2`，从本次输入副本运行 cargo-llvm-cov，并保留本机对象、profraw、源码摘要和重新导出的 LLVM 原件：

```sh
python3 /opt/symphony-env/run.py python3 \
  /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.2/capture.py \
  --repository "$PWD" --output "$PWD/target/gh14/source-capture-final" \
  --target-dir "$PWD/target/gh14/coverage-target" \
  --source-root apps/server/src --input Cargo.toml --input Cargo.lock \
  --input apps --input migrations --manifest apps/server/Cargo.toml
```

141 个生产 callable 全部有精确原生映射；通过同一插件 `reexport` 重新读取原件，得到最大 CRAP **10/1**、最低逐函数行覆盖率 **4/5**、最低逐函数区域覆盖率 **4/5**，无阈值违反或缺失项。测量系列仍为仓库既有的 `measurement-series/v1:b79abd709904548fbed1042b2e1ceb146cc3fedc75bc55744eb475b84cdd1db9`。

早期采集暴露了表达式闭包的精确映射限制及未覆盖的错误分支；已简化生产表达式并加入真实启动超时、许可锁超时、暂停握手竞争、损坏身份与文件写入错误测试，再重新采集。没有修改 collector、Core、基线、签名、来源守卫或阈值。最终摘要生成时再次核对所有采集 Rust 源码 SHA-256 与当前工作区一致。

Angular 22 的生成类型由固定 HTTP collector `cli.cjs generate` 重新生成，随后使用现有 ESLint `--fix` 和 Prettier 规范格式；所有响应/请求类型经该插件 `checkTypes` 与同一 OpenAPI 重新生成结果核对。暂停请求明确要求 `{"pause":true}`，拒绝 false、缺失及未知字段。

## 验收对应与边界

- 双启动：真实服务不同 cwd/端口仍只能一个实例；并发领取只提交一个占用者和一个 Run。
- A03：真实父进程退出后的 setsid 后代持续写入，停止后全部回收；旧监督器丢失、启动身份缺口、PID 启动时间不匹配均保留屏障。暂停/重复停止和启动许可竞争不产生额外写入者。
- A04：新 incarnation 不采纳旧 Run/request/incarnation 事件；即使旧记录已是 Failed，未静止事实仍必须检查。原 phase、工作区及进程身份保留。
- A09 占用部分：验证、交接重试、CI / 问题等待、暂停、重启均不释放；丢失控制行且已有 Run 时也不能推断空闲。慢数据库锁不阻塞 API 和同步 tick。

真实编码、Git worktree/Broker、工作保全、预检/磁盘/额度、交接与合并/取消释放尚未实现，本项 tick 不自动领取 Ready。完整 A03/A04/A09 和产品 0a DoD 不因本记录而完成。

本地 HTTP rehearsal 不故障注入供给数据库去代替宿主的健康 503 采集。完整签名验证、所有 collector 与两个受保护检查由既有受信宿主在**发布后的精确 SHA**运行，状态保持 CI pending；Agent 不持有签名密钥、不自批信任配置。

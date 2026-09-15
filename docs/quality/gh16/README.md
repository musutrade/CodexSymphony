# GH-16 本地验证

基线 `6c885ca444f5d503116fa558419413ef413ae280`（PR #36）。前置 #15 已合并，
其 head `fb4ad6bda8620e08d7c77cd4487f0d2e2ffec168` 的 Harness-Gate 和 Trusted Harness-Gate 均成功。
本次在原 `symphony/GH-16` 工作区继续，保留 `artifacts/gh16-preparation/` 中历史阻塞/操作员恢复记录。
实际沙箱的数据库查询、test 重建丢弃及 dev 持久保留检查通过；Rust 1.97.1、工作区 target 和固定 Gate 可用。

## 验收内容

`apps/server/tests/github.rs` 使用真实 localhost HTTP server、临时生成的合成 RSA key、
独立 PostgreSQL schema 和真实产品 CLI/服务进程，覆盖：

- token 到期刷新、401 一次刷新、精确 repository_ids/最小 0a 权限请求、私仓权限不足；
- suites/check runs 分页、重复同名与发布者冲突、run/suite/attempt/job 精确匹配，旧失败被有效重跑替代；
- statuses 与 Checks 分离，无 status 不阻断 check-run，status-only 仍得到独立事实；
- capped / 缺失 / 异常 JSON、403/404、closed 未合并、开放 PR test-merge SHA；
- 缺能力、策略变更与 stale 拒绝领取，AgentRun 数量为零，不执行模型程序；
- 持久化 60 秒轮询、失败退避和未知时保留旧事实；任何观察都不改业务/Run 或释放占用；
- 启用配置后真实后台观察、只读 CLI、服务退出和传输错误。fixture 拒绝除 token 签发外的所有 POST。

状态/普通 App check 的来源结果可观察；外部触发配置没有可信适配器时，预检保持未就绪。
这不是静默假定能够触发。当前可准备的自动检查路径是策略固定的 GitHub Actions workflow。

## 真实只读回查

[脱敏结果](live-readonly.json) 来自本次宿主 `github_api` 对 `musutrade/disposable` 的 GET：
精确仓库 ID、PR #3 完整 merged/merged_at/head/base、head 的 check runs 和 statuses。
该公开仓 PR 的两条同名 `test-job` 来自不同 suite，statuses 为空；不能把同名结果任意合并。

产品只读入口及配置见 [使用说明](../../github-observation.md)。真实回查使用宿主适配器，
未将宿主私钥交给 Agent 或运行 Rust 产品的真实 App 凭证路径。真实私仓和写路径未验证；
合成私仓 fixture、历史 S2、开发本仓库的 CI 均不冒充这些结果。

## 质量与重现

30 项 workspace 测试通过；305 个生产 callable 全部有精确原生映射，最大 CRAP 10，
最低逐函数行/region 覆盖率均为 80%，没有缺失或超限。
精确命令、退出码、原生测量及最终输入身份见 [local-summary.json](local-summary.json)。
原件保留在工作区 `target/gh16/`；不复制历史签名或修改可信宿主、CI、阈值及 quality policy。
新增 Rust HTTP/JWT/时间解析依赖改变锁文件和采集 pipeline 摘要，使用同一固定 rust-source rc.2
重新采集；宿主仍需针对发布的精确 commit 审核当前输入并完成双检查。

```sh
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/gate.py config check
python3 tools/gate.py secrets
python3 tools/gate.py audit
```

原生覆盖率采集命令：

```sh
python3 /opt/symphony-env/run.py python3 \
 /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.2/capture.py \
 --repository "$PWD" --output "$PWD/target/gh16/source-capture-complete" \
 --target-dir "$PWD/target/gh16/coverage-target" --source-root apps/server/src \
 --input Cargo.toml --input Cargo.lock --input apps --input migrations \
 --manifest apps/server/Cargo.toml
```

保留的修正记录：首次并行运行两套数据库测试遇到 advisory lock 超时；停止重叠运行后专项通过。
collector 不支持几处 vec!/write!/println! 宏以及一个闭包源码 anchor，改为等价标准库/命名函数。
后续采集揭示缺少 branch HTTP fixture、错误分支覆盖及函数复杂度问题，已补 fixture/测试并拆分职责。
这些失败原件未删除或替换。没有前端、真实 GitHub/cloud/通知写入 spike 或门禁策略变更。

本地结果不代表受信宿主/远端 CI PASS 或 0a 整体业务验收。

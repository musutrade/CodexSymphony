# GH-21 本地验收记录

本次实现事务 outbox、条件 push/唯一 PR 对账、暂停恢复、取消关闭和精确合并事实释放。
开发基线为 `5df5eb45f84b2f42e773ece77f7c568e620ea769`；实际验证输入包括未提交改动，
由 [source-inputs.json](source-inputs.json) 逐文件绑定，不能把基线 SHA 当作发布 SHA。
发布身份以后续 PR 的实际 head/tree 为准。原实现、累计预算及历史失败证据均保留。

2026-09-16 实际运行：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `python3 /opt/symphony-env/run.py cargo test --workspace --locked` | 全 workspace 通过，包含真实 Runtime、验证和数据库故障用例 |
| `python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `python3 tools/gate.py config check` | 通过，配置与必需性未修改 |
| `npm run lint`（`web/angular`） | 通过 |
| `python3 /opt/symphony-env/run.py python3 .agent-tmp/run-browser-check.py` | 10 项桌面/手机浏览器测试通过；包装器只创建独立测试 schema 并调用供应的 `e2e.py` |
| `node --check tools/a01-smoke.mjs` | 语法检查通过，未执行外部副作用 |

真实源码采集使用已安装的 `rust-source/0.1.0-rc.3/capture.py`：

```sh
python3 /opt/symphony-env/run.py python3 /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.3/capture.py \
  --repository . --output .agent-tmp/gh21-source-accepted \
  --target-dir target/gh21-coverage --source-root apps/server/src \
  --input Cargo.toml --input Cargo.lock --input apps --input migrations \
  --manifest apps/server/Cargo.toml
```

该入口再次运行全部后端测试，保留并重新导出 LLVM 原生对象/counters，并核对源码函数清单。
同版本 `measure.py` 对这些原生导出计算 [measurement-summary.json](measurement-summary.json)：
690 个生产函数；行覆盖率 **6722/6936（96.91%）**，region 覆盖率
**11818/12971（91.11%）**，最大 CRAP **10/1**；无超限、无不支持/缺失测量。
采集后再次核对所有输入摘要与工作区一致。没有把 MIR 状态机指标替代源码指标。

新增确定性测试覆盖响应丢失、提交失败回滚、重复投递、未知创建、取消/合并竞争、关闭
失败、错误身份、暂停占用及恢复、首次 Ready 准备和新鲜精确合并释放。HTTP 夹具运行
真实 App 客户端、GitBroker 和完整交付服务；真实本地 bare Git 验证新建/fast-forward
成功、非 fast-forward 拒绝且远端 head 不变。这些是开发测试，不冒充真实 GitHub A01。

保全工作区的原始证据：`.agent-tmp/gh21-workspace-accepted.log`、`gh21-clippy.log`、
`gh21-browser.log`、`gh21-source-accepted/{capture.stdout,capture.stderr,bundle.json,raw/}`、
`measurement.json` 和 `summary.json`。早期 PostgreSQL EOF、源码定位及 CRAP 未达标记录
仍保留；最终全量测试和采集分别在供应器确认的新一次性 PostgreSQL fixture 上通过。
持久化 dev fixture 未清空。

`python3 tools/gate.py hook` 曾返回失败：其源码快照未带 npm 可执行依赖，且工作区没有
独立签发的 `hook-state.json`。普通工作区没有签名输入，未复制/伪造这些状态；直接运行的
fmt、Clippy、npm lint 已通过。此记录不宣称 hook 或正式双 Gate 通过。

真实 A01 的入口、输入和产物格式见 [交接文档](../../delivery.md)。此工作区未设置产品
`GITHUB_APP_CONFIG` 或 `RUNTIME_CONFIG`，未运行真实 GitHub 写入 A01；PR/SHA/运行与
验证原始证据留待配置好已授权验证仓的单仓验收任务。开发 PR、浏览器表单测试和脚本化
模型协议测试都不能代替 A01。正式验收服务仍须对实际发布 head 运行精确提交双 Gate。

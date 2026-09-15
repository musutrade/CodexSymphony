# 项目固定的独立测量插件

`packages/` 保留本机组合验收使用的候选包；版本和 SHA-256 由
`.harness-gate/collector-candidates.json` 固定。它们是 Harness-Gate collector，
不修改 Core，不是 Codex 插件，也尚未作为上游正式签名发行。

```sh
python3 tools/install_gate_plugins.py
```

安装器先验证归档摘要，检查已有同版本文件一致后才切换命令链接，保留旧版本。
Node 包使用锁定依赖并禁用安装脚本；Rust 源码插件使用独立版本目录。

本地源码与提交：

| 插件 | 工作树 | 提交 |
| --- | --- | --- |
| Rust source rc.1 | `~/Harness-Gate-rust-source` | `337c363` |
| TypeScript rc.4 | `~/Harness-Gate-ts-crap` | `588e9ac` |
| HTTP contract rc.3 | `~/Harness-Gate-api-contract` | `845dd82` |

## GitHub 保存状态（2026-09-15 核实）

三个候选安装包及摘要已随 [CodexSymphony PR #26](https://github.com/musutrade/CodexSymphony/pull/26)
合并，可从本仓库 `tools/gate-plugins/packages/` 获取。
上述源码工作树均属于 `musutrade/Harness-Gate`；对应的 `feat/rust-source-risk`、
`feat/typescript-crap`、`feat/api-contract` 分支及表中三个提交在本次查询时均未出现在
GitHub，也没有对应 PR。它们已有本地 Git 提交，但尚未推送或合并上游。
安装包已保存不代表源码开发分支已交付，也不代表正式签名发行已发布。

完整运行通过仓库外宿主入口：
`~/.local/share/codexsymphony/gate-host/run --repository ~/CodexSymphony`。
安装插件本身不生成受信请求或批准基线。完整证据见
`docs/quality/complete-local/`；后续插件修改必须使用新候选版本并重新验收。

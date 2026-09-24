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
上述源码工作树均属于 `musutrade/Harness-Gate`，现已保留原提交推送至对应分支：

| 源码分支 | 上游 PR | 本次复测 |
| --- | --- | --- |
| `feat/rust-source-risk` | [#270](https://github.com/musutrade/Harness-Gate/pull/270) | 5 项 AST、12 项测量/协议测试通过 |
| `feat/typescript-crap` | [#271](https://github.com/musutrade/Harness-Gate/pull/271) | 16 项单元、4 项 Core 0.4.5 集成测试通过 |
| `feat/api-contract` | [#272](https://github.com/musutrade/Harness-Gate/pull/272) | 9 项合约测试通过 |

已回读确认三个 PR 的 head 与表中源码提交一致；记录时 PR 均开放、上游 CI 运行中。
源码已推送，不代表已合并或正式签名发行已发布。消费者继续使用当前已审查的候选包摘要。

完整运行通过仓库外宿主入口：
`~/.local/share/codexsymphony/gate-host/run --repository ~/CodexSymphony`。
安装插件本身不生成受信请求或批准基线。完整证据见
`docs/quality/complete-local/`；后续插件修改必须使用新候选版本并重新验收。

## Rust 完整诊断候选 rc.7（2026-09-24）

源码为 [Harness-Gate PR #281](https://github.com/musutrade/Harness-Gate/pull/281)，
提交 `a051da4`。`rust_source` 独立版本为 `0.1.0-rc.7`，与原生 Rust
collector 的版本分别管理。当前候选归档及 SHA-256 固定在仓库清单中，
不自动跟随上游 latest，也不以同版本重打包替换已批准内容。

新版在覆盖率编译和测试前扫描全部批准源码，汇总兼容性错误；取得覆盖率后
汇总可安全检查的映射错误。缺失或歧义仍禁止通过，指标公式和阈值不变。

升级需要完整本地验收及新的宿主批准配置，再同步远端 Gate 和开发环境。
Gate 使用版本目录中的绝对路径，并核对运行文件摘要；插件实现改变会改变
测量身份，旧证据不能用于新实现。旧分支应同步环境清单和门禁配置；不匹配
时明确拒绝，不静默回退。安装包测试通过本身不代表正式宿主已切换。

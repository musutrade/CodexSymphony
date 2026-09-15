# CodexSymphony 开发门禁

2026-09-15 按用户要求接入。Core v0.4.5 + Rust collector 0.1.0-rc.6，
版本以根目录 `harness-gate-version.lock` 为准。本仓库从开发阶段使用门禁；
未来 Rust 平台对受管仓库的 Gate 编排集成仍按方案分期。

## 入口和策略

```bash
python3 tools/gate.py config check
python3 tools/gate.py doctor
python3 tools/gate.py hook
python3 tools/gate.py verify --profile ci --all
```

入口同时检查两个程序的实际版本和已验证发布二进制 SHA-256；目前支持 Linux amd64。
默认使用 PATH，本机已安装锁定版本。其他平台必须先审定并增加对应发布摘要。
GitHub Actions 的 job 名为 `Harness-Gate`，等待独立 App 宿主对精确提交运行同一门禁入口。
main 同时要求该检查（App 15368）和 `Trusted Harness-Gate`（App 4867361），
启用 strict 更新、管理员强制、禁止 force push 和删除。真实远端接入见 `docs/remote-gate.md`。

- 后端：根 Cargo workspace，源码 `apps/`、`crates/`；format、Clippy、compile、tests。
- 前端：`web/angular/`；lint、tests、build。Angular / Material 骨架及接口已建立。
- Rust 源码行／region 覆盖率与 TypeScript 行／函数覆盖率维持至少 80%。
- Rust 已选择独立 `rust-source-risk` 源码分支系列；async 状态机与宏生成控制流
  不计入源码 CC。原 MIR 系列仅作诊断，测量口径不能混用。
- 前后端 `risk.crap` 均要求 **≤ 10**：`operator=le`，精确有理数 `10/1`，
  `required=true`、`on_violation=fail`。策略文件是 `.harness-gate/packs/*/policy.json`。
  当前采用绝对上限，无历史债务豁免；不把 10 作为平均值或只写入提示词。
- full/ci 是完整质量配置；hook 仅部分保证，不能用 hook 通过代替 CI 完整通过。
- 前后端 API 合约策略沿用预设，收集兼容性、破坏性变更和客户端漂移证据。

## 实际状态与剩余交付准备

本机受信宿主已完成一次 **完整隔离门禁 PASS**：秘密扫描、架构检查、7 项执行步骤，
以及 Rust 源码、TypeScript、HTTP 合约三个 collector 的 20 条证据一起通过。
CRAP 上限仍为 10，覆盖率仍至少 80%；没有降级 required、跳过失败或复用旧签名。
详细记录见 `docs/quality/complete-local/README.md`。

复现入口是仓库外安装的受信宿主：

```sh
~/.local/share/codexsymphony/gate-host/run --repository ~/CodexSymphony
```

它创建源码副本和独立数据库，固定输入与已批准策略，隔离执行真实采集，签发新请求，
再执行完整 `tools/gate.py verify --profile ci --all`。批准文件、基线和宿主程序在仓库外；
测试命名空间看不到宿主密钥，源码及运行时信任文件在验证期间只读。
私钥在采集结束后才创建，签名完成即删除；每次运行使用新 nonce。
宿主通过独立 broker 在测试挂载之外持久化 nonce，重启后仍拒绝重复请求。
Core 0.4.5 的完整 verify 未配置持久化 replay state，已提交
[上游 Issue #269](https://github.com/musutrade/Harness-Gate/issues/269)；当前防护由宿主补齐，
不能将裸 Core verify 描述为已经具备同样的跨进程防重放保证。
`--profile full` 使用相同要求重新签发对应 profile 的输入。

`.harness-gate/runtime/` 不进 Git。直接在未供给受信输入的普通工作区执行完整 verify
仍会阻断，这是预期行为；不要把某次运行的 state 或签名请求复制回来长期使用。

- Rust 源码插件：0.1.0-rc.1；MIR 仍仅作独立诊断。
- TypeScript 插件：0.1.0-rc.4，已接入生产文件／函数清单及共享产物目录。
- HTTP 合约插件：0.1.0-rc.3，检查真实 200／503 响应、完整支持范围内的客户端清单、
  生成类型来源及初始兼容性基线。未知合约或客户端语法会拒绝采集。
- 插件安装包在 `tools/gate-plugins/packages/`，摘要在 `collector-candidates.json`；
  本机与 CI 使用 `tools/install_gate_plugins.py` 安装，旧版本保留。它们仍是候选发行，
  不冒充 Harness-Gate 上游正式签名发布。

远端受信输入和真实 CI 已接通：准备 PR #26 的 head 与合并后的 main 均完整通过，
每次重新采集 3 个 producer、20 条 evidence。宿主程序和批准文件不由 Actions 或 PR 自行生成。
Symphony 使用独立服务、台账、工作区与 Codex 文件系统命名空间；环境验收为 #27 / PR #28。
实际状态和原始记录以 `docs/quality/remote-environment/` 为准。
已有 0a #12–#25 均为 symphony-ready；安装器额外要求环境验收标签，业务队列需另行放行。
不得在不可信 PR job 中自动 bootstrap 策略、放入签名私钥或自批新的测量系列。

原 MIR collector 的独立 ratchet 路径未启用，本项目继续使用 Core 完整配置；
API 的初始合约基线与 MIR 历史风险基线是不同输入。

## 本机安装记录

安装器来自固定 tag v0.4.5；安装时通过 SHA256、Sigstore 签名及证书身份验证。
本机默认命令位于 `~/.local/bin`，指向版本目录；旧 v0.4.2 目录保留。
项目专用副本位于 `~/.local/share/codexsymphony/harness-gate/bin`。

- Core Linux amd64：`70721282c751826ed4d57e14bd7de9516e73e833aa058d758dbd2154c0aa5e10`
- Rust collector Linux amd64：`520e3fc0fa4938694a10abc504e3a5d0f164cf2bf9314cd4a00c5d613bbbf861`
- v0.4.5 install.sh：`c4a6f47bd8a94ff871966102b36527fccd92e5367c9e46174d1f9a032b49364c`

这些摘要用于重现本次已验证的发布输入，不将安装成功等同于项目质量验收成功。

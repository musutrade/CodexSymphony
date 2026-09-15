# 远端受信门禁与开发控制器

GitHub Actions 的 `Harness-Gate` 不执行带凭据的项目代码。仓库外的宿主服务
读取 Actions 的精确 head SHA 与 run ID/attempt，拉取同仓源码，核对批准的工作流、
门禁输入和依赖锁，然后调用已安装的完整隔离门禁。每次重新采集、签发请求和消费 nonce。

宿主使用独立 GitHub App `my-disposable-bot`（ID 4867361）发布
`Trusted Harness-Gate` check。Actions 只接受 App ID、slug、head SHA 和
external ID（run ID/attempt）全部匹配且 conclusion=success 的结果。
缺失、失败、超时、旧 attempt 或其他发布者均不能成功。

main 需要同时保护两个检查：`Harness-Gate`（GitHub Actions）和
`Trusted Harness-Gate`（上述独立 App），并禁止 force push、删除与管理员绕过。
Symphony 继续检查 `github-actions` 发布的 `Harness-Gate`，GitHub 分支规则额外
强制独立 App 检查。普通 GitHub 授权创建的同名结果不具有该 App 身份。

## 安装和授权输入

```sh
python3 tools/install_remote_gate.py
```

此操作安装固定版本的宿主脚本、配置和 `codexsymphony-remote-gate.service`，
不自动启动。批准配置在 `~/.local/share/codexsymphony/remote-gate/`；
App 私钥仍在工作区之外，运行时 installation token 仅驻留宿主内存。
App 需安装到本仓库，具有 Checks 读写、Actions 只读、Contents 只读、Pull requests 只读。
宿主签发 token 时只请求这一个仓库与上述权限，不使用 Agent 的 GitHub 凭据。

确认 App 安装和权限后启用服务；执行记录保存在 `remote-gate/jobs/<run>-<attempt>/`，
完整测量原件仍在 `gate-host/runs/`。服务重启会把未完成的旧 claim 标记为中断失败，
需重新运行 Actions 获得新 attempt；不会把旧运行直接变成成功。

当前仅支持同仓 PR、main push 和手动工作流，不支持 fork。依赖使用本机已准备的
锁定安装；依赖锁、工作流、门禁或受信入口修改须重新审定，不能由 PR 自批。
运行源码和依赖在采集命名空间只读，签名阶段与测试分离。

## 启用验收

- App 实际安装权限、独立 check 发布和发布者身份回读。
- 准备分支 PR 的真实完整 PASS；按实际 App ID 配置并回读两个必需检查。
- 独立 Symphony 服务、工作区、环境和台账；真实 Issue 经 host github_api 创建 PR。
- 控制器观察精确 head 的 CI、按保护规则合并、交接并关闭测试 Issue。

这些结果必须保留实际链接后才能标记完成。脚本测试或本机历史 PASS 不代表远端验收完成。
`WORKFLOW.lifecycle.md` 描述的是现有 Elixir 开发控制器，不改变 V10.2 产品范围。

## 独立 Symphony 服务

`python3 tools/install_symphony_development.py` 准备独立服务、环境和稳定台账路径，
不复制 Harness-Gate 的历史台账，也不自动启动调度。当前版本固定 Codex 0.154.0。
`tools/symphony/codex_sandbox.py` 在另一个文件系统命名空间内启动 app-server，
仅挂载该 Issue 工作区、模型认证目录与构建工具；App 私钥、GitHub CLI 凭据、
宿主批准文件及交接台账不挂载。GitHub REST 仍由外部 Elixir 动态工具执行。
`.git` 在 Agent 命名空间中只读，模型认证目录不等同于 GitHub 凭据隔离。
当前预检已确认 Codex 版本和 App 私钥／门禁批准文件不可见；真实模型交接待远端验收。

Codex 认证复用项目已有的独立 CODEX_HOME；官方说明见
[Codex authentication](https://learn.chatgpt.com/docs/auth)。

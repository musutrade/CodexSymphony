# 远端受信门禁与开发控制器

GitHub Actions 的 `Harness-Gate` 不执行带凭据的项目代码。仓库外的宿主服务
读取 Actions 的精确 head SHA 与 run ID/attempt，拉取同仓源码，核对批准的工作流、
门禁输入和依赖锁，然后调用已安装的完整隔离门禁。每次重新采集、签发请求和消费 nonce。

宿主使用独立 GitHub App `my-disposable-bot`（ID 4867361）发布
`Trusted Harness-Gate` check。Actions 只接受 App ID、slug、head SHA 和
external ID（run ID/attempt）全部匹配且 conclusion=success 的结果。
缺失、失败、超时、旧 attempt 或其他发布者均不能成功。

main 已同时保护两个检查：`Harness-Gate`（GitHub Actions）和
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
`tools/symphony/trusted_environment.py` 只负责部署级开发环境边界，省略 GitHub 凭据、签名密钥和控制面状态的挂载。
环境内 Codex 使用 danger-full-access；普通 Git、构建、数据库、浏览器和真实 Runtime 测试直接运行。
不要求嵌套命令沙箱、静态网络白名单或逐测试宿主入口。正式 Gate 继续使用独立执行和签名服务。
此部署采用已有的外层环境启动器；产品不要求所有部署使用同一种隔离技术，也不承诺接管不可信代码。
安装后运行 `python3 tools/symphony/check_environment.py`，通过真实 command/exec 检查普通命令、
本地 Git 可写、工具版本及服务凭据不可见。该检查不替代项目测试和精确提交双 Gate。
完整边界及迁移说明见 [可信开发环境](trusted-development.md)。旧环境验收记录仅是历史证据。

## CI 范围与过期任务（2026-09）

同一 PR 的新 Actions 运行取消旧运行，宿主在出队、准备后、执行期间及发布前
核对实际 Actions 状态。执行中的旧 PR 按约 15 秒间隔检测，终止整个启动进程组，
记录 CANCELLED，不能发布成功。API 错误关闭当前执行并报告失败。main push 与手动
运行不参与 PR 自动取消；手动 workflow_dispatch 始终执行完整门禁。

文档快速检查由安装在宿主的 `ci_policy.py` 决定，PR 不能选择范围。当前白名单只有
根 README、综合方案与 `docs/evidence-lifecycle-impact.md`。须在最近 100 个宿主回执中
找到**相同批准配置、完整通过、且为当前提交祖先**的基线，并对基线至当前提交的
全部差异检查；混合代码、未知文件、WORKFLOW/AGENTS、删除、改名、可执行文件或
符号链接均回到完整门禁。首次升级无匹配基线，也执行完整门禁。

快速检查验证普通 UTF-8 文件、每文件 2 MiB 上限、冲突标记与 diff 空白错误。
它不验证文档语义、外链或产品功能，也不宣称重跑覆盖率。独立 App 回执明确标识
`documentation` 范围，绑定当前 SHA、Actions attempt、批准配置摘要、完整基线
SHA/attempt/report 摘要。main 上仅文档差异可采用同一规则。白名单扩展必须审查
文件是否被构建或执行器消费。

文档结果是小 JSON，不复制 node_modules、不生成 Rust target。保留策略单独清理
其源码克隆；文档通过不淘汰完整测试基线。取消的失败尝试可在同一 PR 后续完整
通过后归并。构建缓存仍按现行容量策略清理，本次不引入无界缓存。

升级需安装经审查的 remote-gate 版本及 evidence-archive 版本；仅修改 Actions YAML
不会更新宿主。保护文件发生变化的本次 PR 本身必须通过新宿主的完整门禁。

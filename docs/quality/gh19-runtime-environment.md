> 历史验收记录：执行边界已于 2026-09-16 改为可信开发环境。本文旧沙箱、白名单和专用验收入口不再是当前要求；以主规格及 docs/trusted-development.md 为准。历史结果保持原样。

# GH-19 Runtime 环境恢复（2026-09-16）

原阻塞在 initialize 之前退出。仅设置 sqlite_home/log_dir 不足以隔离 Codex 的
全部可写运行状态；共享 CODEX_HOME 在 command/exec 内只读。为测试子进程创建
工作区内独立 CODEX_HOME 后，真实 app-server 可以握手并创建 thread。原 stderr
中的 bubblewrap 警告仍可能存在，它不等于 initialize 失败。

## 实际验证

在分配的 GH-19 命令沙箱内执行：

```sh
python3 /opt/symphony-env/runtime_smoke.py
```

- Codex：`codex-cli 0.154.0`；UID `1000`。
- 工作区、进程 cwd、thread cwd：`/home/gem/.local/share/codexsymphony/workspaces/GH-19`。
- 被测工作区 Git HEAD：`cba6a54475209405b64ee6bca0720444cd054be6`（保留原准备阶段基线）。
- 安装探针 SHA256：`26d534c36006e816f3a2e11fd7b15d4e004299362e154166676b081b5af9a525`。
- initialize/initialized、thread/start、动态工具请求/回复、turn/interrupt：PASS。
- `.git` 只读，宿主凭据与 Gate approval 不可见。
- provider 为当前命令网络命名空间内的固定 Responses fixture；真实模型调用 0，
  不复制/链接认证、不扩大网络许可、不关闭沙箱。
- 临时运行状态随退出删除；探针只输出小型 JSON，异常输出限制为末尾 4000 字符。
- 宿主复盘记录：`symphony/gh19-runtime-recovery/runtime-smoke.json`。

## 效力边界

这证明 GH-19 所需的 app-server 协议 smoke 可在现有隔离内执行。它不证明 Rust
产品 Runtime 已实现或通过验收，不证明生产模型联网认证，也不证明更深层的
shell 沙箱能够执行。后续产品测试应调用实际 adapter，并分别验证这些边界。

通用环境模板为现有和新 Issue 安装同一探针；状态目录按测试隔离并清理。
GH-19 恢复时保留原阻塞报告及累计预算，不重置尝试次数。

## 命令执行复核（2026-09-16 05:42 UTC）

GH-19 后续在真实嵌套命令执行阶段再次阻塞；上面的协议 PASS 不构成此次
解除阻塞的依据。通过已部署的 `gh19-environment/preflight.py`，在实际
Agent `workspaceWrite` 命令沙箱内重新执行保留的两份探针：

- `target/gh19-preflight/runtime_command_probe.py`：外层退出 1，内层
  `/bin/true` 退出 101；`/tmp/codex-bwrap-synthetic-mount-targets-1000/lock`
  只读。
- `target/gh19-preflight/operator_runtime_command_probe.py`：使用工作区内
  独立 `CODEX_HOME` 和 `TMPDIR` 后，内层退出 1；bubblewrap 无权创建
  新命名空间。历史 `preview.json` 还记录了 `NETLINK_ROUTE` 创建失败。
- 同一实际命令环境的独立检查确认 `NoNewPrivs=1`、`Seccomp=2`、
  一个 seccomp filter、AppArmor `bwrap//&unpriv_bwrap (enforce)`；
  创建 `AF_NETLINK / NETLINK_ROUTE` socket 返回 `EPERM`。这些是观测事实，
  尚不能将所有命名空间错误单独归因于某一个安全层。
- UID 1000、Git 只读、宿主凭据和 Gate approval 不可见检查通过；
  部署的受管网络仍启用，域名许可保持原值。未修改隔离或认证配置。

本次证据保存在宿主
`symphony/gh19-command-recovery/20260916T054113Z/`，包括原始与私有临时目录
探针结果、隔离检查、复核摘要、原台账备份和源码前后 SHA256 清单。
15 个已修改或未跟踪的源码文件内容一致，Runtime 实现完整保留。
GH-19 继续为 `blocked`，运行任务数为 0；累计预算及两次运行计数未重置。

当前没有完成命令执行修复。私有临时目录只解决第一层写入失败，无法证明
嵌套命名空间和网络初始化可用。后续必须在保留隔离策略的执行方案下，重新
取得原恢复条件要求的实际命令 PASS，才可解除阻塞并继续产品 Runtime 验收。

## 后续定位与待确认的执行方案

内核日志进一步确认：05:41:32 的私有临时目录探针执行期间，
`apparmor="DENIED" operation="capable" profile="unpriv_bwrap" comm="bwrap"`
拒绝 `capability=21 capname="sys_admin"`。日志已归档为上述证据目录中的
`kernel-apparmor-denials.log`；`policy-identity.json` 记录相关 AppArmor 和
网络配置 SHA256。系统 `bwrap-userns-restrict` 对子进程叠加
`unpriv_bwrap`，其中明确 `audit deny capability`。因此仅修复临时目录
不能解决本次嵌套执行失败。此轮未修改系统 profile、sysctl 或受管网络。

可评估的替代方案是复用现有 preparation 验收的宿主固定入口模式，让
产品 Runtime 验收从控制器所在安全层启动独立沙箱，而不是从 Agent 的
`command/exec` 子进程再启动沙箱。这是验收拓扑变化，尚未实施，也不满足
原嵌套探针恢复条件；须先确认接受该路径，再实现与验证：

1. 宿主入口只接受固定验收动作；不接受任意命令、环境变量、工作区路径或
   Codex 配置。使用受审查快照和哈希绑定，拒绝工作区替换或过期收据。
2. 复用固定 Codex 版本、工作区、只读 Git、隐藏宿主凭据和受管网络白名单；
   每次验收建立独立运行状态，避免读取共享认证。测试进程也必须受隔离。
3. 先验证真实 `command/exec` 及允许域、拒绝域、直连拒绝，再通过实际 Rust
   adapter 验证产品行为。Python 环境探针只能证明环境能力。
4. 独立保留原嵌套 FAIL 与新路径结果，记录获确认的新恢复条件；新路径
   未通过前不解除 GH-19 阻塞，不重置累计预算，不改写已有 Runtime 源码。

如果必须保留原嵌套方式，则仍需解决 AppArmor 能力限制及受管网络的嵌套
兼容性；当前没有已验证且保持所有既有约束的配置修复。

## 已授权修正与复验（2026-09-16 05:59 UTC）

用户确认按设计复核建议处理，恢复条件改为符合产品部署结构的独立 Runtime
命令环境检查；原嵌套 FAIL 保留，不再将任意沙箱嵌套作为产品必需能力。

已安装固定 `runtime-command-readiness` 操作。Agent 执行
`python3 /opt/symphony-env/runtime_command_readiness.py`，宿主通过同一固定
启动器启动独立 app-server，挂载空白临时状态而非共享认证；固定检查不接受
调用者命令、路径或配置参数。进程退出清理临时状态。

实际复验：命令退出 0，Codex 0.154.0、UID 1000、Git 只读、宿主凭据隐藏、
共享认证不存在均通过；允许域返回 200、禁止域有明确代理拒绝 403、公共 IP
直连返回 ENETUNREACH。有效网络配置前后相同并匹配已部署许可集合。
未修改 AppArmor、sysctl 或网络白名单；模型调用为 0。

GH-19 的 `process.rs` 补入 Runtime 子进程 `env_clear` 加明确许可集合，
保留部署工具/网络设置，使用每 Run 独立 CODEX_HOME/TMPDIR。新增真实
supervisor 子进程测试，以合成值验证 GitHub/GitLab/OAuth、自定义凭据及
数据库等非许可变量不会继承，工具 PATH 及临时目录仍可用。该测试通过；
其余 14 个既有改动文件 SHA256 未变化。开发环境 Python 测试 30 项通过。

宿主证据目录：`symphony/gh19-command-recovery/fix-20260916T055523Z/`。
其中包含原 process.rs、原阻塞报告、独立命令收据、Rust 测试结果及恢复说明。
这是环境准备恢复，不是完整 Rust Runtime 产品验收；后续仍须完成实际
adapter 集成、行为验收及产品质量门禁。

### 恢复时旧 handoff 的遗漏与修正

06:01 的首次恢复只改了宿主台账，遗漏工作区 `.symphony-handoff.json`。
AgentRunner 在启动 app-server 前读到旧 blocked 声明，701ms 后再次阻塞；
没有新增 token，但消耗了第三次运行尝试。此前仅依据短暂 running 状态
报告恢复成功不充分。

旧声明已移至 `symphony/gh19-command-recovery/stale-handoff-20260916T060404Z/`，
原台账、预算及声明 SHA256 均保留。用户明确批准仅为 GH-19 追加一次运行；
累计计数保持 3 后正常递增至 4，没有重置 token 或时间。串行领取该次任务后
立即将部署默认运行上限恢复为 3，其他任务不增加额度。

06:04:53 再次领取；06:05:04 已确认真实 app-server 会话、首轮及 Agent 消息
事件，证据保存在上述目录 `live-session-state.json`。恢复复核必须同时检查
宿主台账、工作区旧 handoff 和真实会话事件，不能只看瞬时 running 数量。

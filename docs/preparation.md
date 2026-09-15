# 执行环境预检与磁盘保护（GH-17）

预检事实不改变 Requirement 验收状态，不把验证失败改成 AgentRun 失败。
当前 Runtime 编码、额度结算和远端 GitHub 写交接仍未接通，Ready 不会因此启动模型。

## 平台入口

`preparation_service::prepare` 接收平台持有的 Launch、Broker/Workspace、控制面目录、
固定 adapter 路径及部署配置。先验证可持久化，再保存尝试，验证 Broker 的 worktree
归属，调用 `tools/preparation/app_server.py`。该 adapter 只使用 initialize、
configRequirements/read 和 command/exec，不发送 turn/start。

部署配置来自管理员控制的快照，不能来自仓库或 Agent 工具参数：

- `launcher`：与后续 Agent 一致的已审定启动器及有效环境；`uid` 是实际执行身份。
- `deployment_identity`、`network_identity`：部署身份及有效 network JSON 的 SHA256。
- `allowed_domains`、`allowed_url`、`denied_url`：实际部署集合和固定探测目标。
- `dependencies`：固定能力命令及期望输出；`writable_paths` 必须包含构建、临时目录。
- `probe_path`：Agent 可读的已审定 sampler；缺省使用 adapter 同目录文件。

探针启动命令必须与 Launch 中的 app-server 命令完全一致。
Rust 服务固定 Core 0.4.5 的版本和 SHA256、Codex 0.154.0，并覆盖配置中的工具锁。
Sampler 同时解析默认 PATH 和 Cargo 优先 PATH，在真实沙箱中执行能力、写入/read/fsync、
静态网络放行、明确策略拒绝和直接公共 IP TCP 绕过探测。控制侧解析 IP，沙箱 DNS
不可用不算绕过已阻止；普通代理变量、一般 403、超时或连接拒绝也不算强制隔离证明。
前后有效网络配置不一致、身份未知或声明超出静态集合均拒绝领取。空声明仍可访问部署集合。

服务将输入及 stdout/stderr 保存在控制面独立 `.preparation-<id>` 目录；原始失败不删除。
探针最多运行 110 秒，退出时停止探针进程组；结构化输出最多读取 1 MiB。
不要把凭据放入部署探针参数。Broker 原件、探针证据与候选源码分开保存。

## 持久化与恢复

`preparation_record` 保存精确 Launch、需求 Revision、实际阶段、证据和固定重试记录；
`preparation_history` 追加尝试及恢复原因。尝试在调用前提交，崩溃后的未知执行不自动重放。
最多初次加两次自动重试，退避 30/120 秒，探测期限 600 秒。超限置一条聚合 `todo`，
失败 code/phase/detail/exit_code/evidence 与累计尝试仍可查询；不扣代码修复次数。
配置身份不符立即要求明确处理。后续额度和基础设施任务复用该记录，不另建重试系统。

`authorize_retry` 是平台内部的显式恢复入口，调用者须先确认旧探针已停止、核查原输出并
持久化用户授权原因；它保留累计历史、恢复原阶段，不清除全局或需求暂停。
过期成功证据也须重新授权并采样。已领取 Run 不能被此入口重新准备。
领取要求证据不超过 60 秒，并匹配精确 Launch/Revision；重复完成不能覆盖成功证据。

平台不改 `/etc/codex/requirements.toml`。网络配置变更须先停止所有受影响执行，由部署
管理员授权更新，回读并实测新的 allow/deny/direct-IP 结果，再明确恢复。开发环境安装器
的固定验收操作不是产品管理员授权接口。

## 磁盘和外部动作

控制面与工作区均保留 256 MiB。检查含可用容量、真实文件写入/read/fsync、目录 fsync，
以及 PostgreSQL 提交写探针。ENOSPC、卷不可访问或持久化故障设置独立 storage_guard；
数据库自身不可写时由进程内锁存补足，重启仍经过原有恢复屏障。

领取、启动许可和 `run_store::actions_allowed` 都检查存储。后续每次模型继续和 GitHub
写操作必须走此许可入口，不能绕过；远端写入仍未实现。协调器仅在存储健康时续写
supervisor 心跳，15 秒无心跳停止后代，无需写 stop 文件。该检测不是零延迟停止保证。
保护触发后仍可消费停止回执，防止恢复死锁。`storage::recover` 要求重新验证存储、
所有 Run 已静止及明确的操作员恢复；不清用户暂停、不删原件、不释放占用。

## 验证边界

真实锁定沙箱验收通过宿主固定操作
`python3 /opt/symphony-env/product_preparation_acceptance.py`，原件只读挂载，绑定
adapter/sampler SHA256 和新 sample_id。覆盖正常、缺依赖、错误能力、工具锁不符、
只读 target、未知网络身份，全部零模型调用。这只证明相同部署路径上的产品 Python 探针。
Rust 集成使用真实 Git/PostgreSQL 与受控 adapter 输出验证领取和持久化错误；受控时钟
验证重试与暂停。`/dev/full` 验证 ENOSPC，真实 supervisor 验证停止与原目录保留。
全产品到 PR 验收、未来模型继续/远端写实现和精确发布提交的受信 Gate 由对应后续边界负责。

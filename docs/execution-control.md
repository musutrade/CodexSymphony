# GH-14：执行控制与冷启动屏障

本项对应综合方案 4.4、5、6、14、21 章与 A03 / A04 / A09 的执行控制部分。
Requirement 仍是业务真相；agent_run 和 run_event 记录执行事实。停止证明不代表工作保全、验证通过、GitHub 交接成功或业务完成。

## 当前运行行为

- 服务使用固定的 `/tmp/codexsymphony-controller.lock` 文件持有进程生命周期锁；更换 cwd、数据库或监听端口不会创建第二个控制者。不要删除或替换锁文件。当前部署和验收为 Linux x86_64 单主机，不能用独立挂载命名空间启动产品副本。
- `execution_control` 单行保存当前 Requirement、全局暂停、本次控制面 incarnation 和恢复屏障。占用覆盖验证、交接重试、CI / 问题等待与暂停。**本项没有释放槽位的入口**；合并/取消的保全及外部对账接入后续交接任务。
- 领取、Requirement → Running 和包含不可变命令/工作区身份的 Run 启动意图同事务提交。领取与原有评审、撤回、安全撤销共享事务互斥；只看队首 Ready，不跳过已暂停或授权被撤销的队首。数据库还约束每个 Requirement 至多一个活跃 Run。
- API 启动前登记新 incarnation 并关闭恢复屏障。tick 只启动至多一个后台恢复任务，不等待数据库查询、文件操作或模型调用完成；API 不等待该恢复任务。事务锁有 500ms 超时，执行事务语句有 2s 超时。
- 产品 tick 当前只进行进程恢复/停止观察。`reserve_prepared` / `start_reserved` 是内部适配边界，真实测试 fixture 使用它们；未连接自动领取或任意命令执行 HTTP 入口。工作树/Broker、工作保全、预检、磁盘和累计额度尚未齐备，`coding_ready` 始终为 false。

## 启动与停止事实

平台的 `EXECUTION_DIRECTORY` 默认为控制面 cwd 下 `.local-data/execution`；实际部署应固定为持久目录，并放在所有 Run 工作树之外。没有 Run 时只读启动不创建此目录。准备适配器负责建立持久根目录，并提供每 Run 独立工作树及身份；本项测试使用独立合成目录，不宣称完成真实 Git worktree 验收。

1. 数据库先提交 Run、revision 引用、request、incarnation、工作目录/身份和完整启动输入。
2. 平台在专属 Run 目录写入并 fsync 启动日志，再启动同一服务二进制的内部 `--supervise <Run目录>` 模式。它在创建 Tokio runtime 前进入单线程监督器。
3. 监督器建立 subreaper、写入 PID / 进程组 / `/proc` 启动 tick / boot ID，随后等待平台许可。平台将此身份写入数据库并再次检查暂停、incarnation 与安全撤销，才写入启动许可。
4. 监督器只启动一个命令，并接管孤儿后代；父命令退出、double-fork 或 `setsid` 均不构成整个执行组停止。停止时用 pidfd 向直接后代发 SIGKILL，回收后继续处理被接管的更深后代。
5. 只有 `waitpid` 确认 ECHILD 才写入包含完整身份的持久化静止凭据。单次启动标记禁止已结束监督器重复消费旧许可。Run 在当前阶段标为 Interrupted，工作保全仍待下游接入，不标为 Succeeded。

监督机制依据 Linux [subreaper](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html) 和 [pidfd](https://man7.org/linux/man-pages/man2/pidfd_open.2.html)。停止只针对该监督器未回收的子进程，PID 不会在打开 pidfd 前被本线程回收重用；打开后通过句柄发送信号。

## 恢复及故障边界

冷启动检查所有 `quiescent=false` 的 Run，**不以是否存在 Running 行决定安全**。先持久化停止意图，再向旧监督器发停止标记；启动后尚未记下 PID 时，从专属启动日志补回身份。没有身份或静止凭据则继续阻塞，不能从 PID 不存在推导全部后代已停止。

监督器自身被杀、日志缺失/损坏、PID 启动时间不匹配、不可终止进程、数据库或文件持久化失败，都不会释放占用或启动下一项。恢复保留 Run、工作区、原始进程身份、phase、暂停和 blocker。缺少身份的崩溃窗口可能需要人工核实所有写入进程并恢复可信材料；本项没有“强制清除占用”或伪造静止凭据的接口。

新 incarnation 拒绝旧 Run/request/incarnation 事件推进；所有事件保留在 run_event，`accepted` 仅说明是否匹配当前活动上下文，不是完成声明。验证和 GitHub 状态尚未接入，事件不会改写 Requirement 或终结 Run。

0a 继续使用本人可信仓库和同 UID 边界；监督器日志是平台控制资料，不能交给工作树写入者修改。这不是恶意同 UID 代码隔离，独立执行 UID 按既定 0c 任务实现。

## Localhost API

所有路由继承已有 Host / Origin / CSRF 保护，写入需合法 Origin 和 `X-CodexSymphony-CSRF: 1`。

| 请求 | 行为 |
|---|---|
| `GET /api/execution` | 当前 Requirement 占用者、全局 paused、进程恢复屏障与明确未就绪的编码原因 |
| `POST /api/execution/pause`，JSON `{"pause":true}` | 先持久化全局暂停和现有 Run 停止意图；随后由 tick 停止后代 |
| `POST /api/requirements/{id}/pause`，JSON `{"pause":true}` | 暂停指定项；未领取队首也不能被跳过；重复调用幂等 |

暂停接口成功只表示意图已持久化，不宣称所有进程已停止。自动恢复不会解除用户暂停。完整用户恢复/保全和后续界面仍由相应任务接入。

## 本项验证

`apps/server/tests/execution.rs` 使用独立合成目录、真实受控 shell/setsid 后代与 disposable PostgreSQL，覆盖并发领取、暂停队首、撤销授权、不可变启动输入、后代写入、启动记录窗口、运行中换 incarnation、监督器丢失、PID 身份不匹配、非 Running 未静止记录、旧事件归档、慢查询与 API 响应。`tests/startup.rs` 启动真实服务并验证不同 cwd/端口的第二实例被拒绝。

运行命令和结果见 [GH-14 验证记录](quality/gh14/README.md)。此处不宣称真实编码、候选保全、额度、Git worktree、PR、完整 A03/A04/A09 或产品 0a 整体验收完成。开发仓库的双 Harness-Gate 仍由宿主对发布后的精确 SHA 执行。

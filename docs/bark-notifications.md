# Bark 行动通知（GH-74）

默认关闭：不运行 timer，或私有配置内容为 `{"enabled":false}`。主服务不读取 Bark key、不发送通知，也不启动通知器。通知器是管理员独立安装的 Python 3 程序，读取 PostgreSQL 已提交的 `notification_action` 投影，将去重/投递结果保存到宿主私有 SQLite。主服务和模型调用不等待投递，不扣模型预算。

## 行动与持久语义

投影与待办箱使用相同业务事实：暂停、失败、问题等待回答/恢复、当前 Run 阻塞、准备/验证/交付阻塞、取消收尾及存储/证据清理。每个来源独立生成 SHA-256 identity；同任务两个问题不会互相吞并。问题版本、有效失败原因、任务 revision、显式恢复版本改变时产生新 identity；重复事件相同 version、重复扫描及普通进度不产生新通知。正常完成不推送。

业务数据库只有新增只读 view（迁移 0024），不重写草稿、授权、用量或保存事实。投影的 key 不包含可读问题、错误文本或凭据。通知正文使用固定描述，链接仅由受限配置的 HTTPS 应用 origin 和正整数任务 ID 组成 `/requirements/ID`。点击只导航，不附带 token、答案或执行命令。未登录由既有登录守卫返回原任务；处理仍须当前版本及用户确认，旧通知不会重新提交旧决定。

同一个业务库/接收设备必须使用**同一持久 state_directory**。POSIX 文件锁串行化并发通知器；SQLite 主键去重，`synchronous=FULL`，每次发送前先提交次数和未知结果。数据库读取失败不会删除待办或取消现有投递。已经处理/变化的 pending 通知变为 obsolete；发送前重新读取业务事实。最后一次检查与网络发送之间仍可能发生处理，收到这类旧提醒时任务页展示当前事实。

每条通知最多 3 次尝试（含初次），退避 30、120 秒，从首次观察起 600 秒 deadline。每次 HTTP 子进程有 10 秒硬超时；重启保留次数、下次时间和 deadline。4xx（除 429）、重定向或无效成功响应终止；429/5xx、网络错误或超时有限重试。HTTP 200 且 JSON `code=200` 只表示 Bark 服务接受，不表示手机收到或用户已处理。

**不是 exactly-once，也不保证 at-least-once。** Bark 未提供本适配器可用的幂等确认协议；接受后响应丢失/本机结果落盘前崩溃，有限重试可能重复。超限后停止，待办仍在应用。`delivery.sqlite3` 同时保存当前状态和逐次 `attempt_result`，不保存 key、请求正文、原始响应或原始异常。不要删除该库“重试”：会丢去重记录。备份时用 SQLite backup API 或停止 timer 后复制，并与对应业务数据库一同保留；恢复演练先禁用 timer 和外部写。回退主服务无需删除 view 或通知 ledger。

## 宿主安装与权限

以下是管理员在最终集成阶段执行的部署步骤，本 Issue 的本地验收不会修改生产服务、Tunnel 或真实凭据。

1. 从通过精确版本双 Gate 的发布源，将 `apps/notifier/bark.py` 安装为 `/opt/codexsymphony-notifier/bark.py`（root 拥有、0755，Agent 不可写）。安装 Python 3 与 PostgreSQL `psql` 客户端，记录实际绝对路径及版本。无需第三方 Python 包。
2. 创建独立系统用户/组 `codexsymphony-notifier`。创建 `/etc/codexsymphony-notifier` 和 `/var/lib/codexsymphony-notifier`，归该用户所有、0700；配置文件 0600。将这两个目录加入编码及验证的 `private_paths`（模板已包含），且不得位于任一允许挂载之下。即使暂不开通知，也须为使用新边界模板的实例创建这两个空私有目录。通知器、数据库配置和 ledger 不放工作区或 Runtime home。
3. 使用受控 DBA 通道创建专用 LOGIN role，仅授予实际数据库 CONNECT、应用 schema USAGE、`notification_action` SELECT；不授予业务表、账号表、DML、创建对象或其他角色成员资格。使用当前平台部署要求的 TLS 数据库连接；只给通知器私有配置连接参数。不得复用 Agent 的项目测试数据库角色。确认该角色读取 view 成功、读取/更新 requirement 失败。
4. 在宿主建立 `/run/netns/codexsymphony-notifier` 独立网络命名空间及独立 egress 规则，允许批准的 Bark HTTPS 服务、DNS 与上述只读数据库；Agent/候选验证不能加入该 namespace，也不能修改路由/防火墙或访问通知器凭据。不要把通知器做成 Agent 可调用的 HTTP 代理。systemd 模板硬性使用 `NetworkNamespacePath`，缺失即拒绝启动，不回退到共享网络。具体路由、CA/DNS、数据库可达性由宿主管理员按实际部署设置，未验证前不得启用。
5. 管理员在受限终端/秘密管理通道填写配置；禁止把真实 key 放 shell 参数、环境变量、URL、聊天或仓库。Bark iOS 安装后，在 App 中取得所属设备 key；只提取 key 到下述 `device_key` 字段，不使用 App 示例的带 key URL。官方 JSON body 协议见 [Bark 文档](https://github.com/Finb/Bark/blob/master/docs/en-us/tutorial.md)。公网端点为 `https://api.day.app/push`；自建服务也必须使用批准的 HTTPS `/push`，不允许 query/fragment/URL 用户信息，不跟随重定向、不使用继承代理。

启用配置字段如下（占位符不是可用凭据；只在受限宿主填写）：

```json
{
  "enabled": true,
  "application_origin": "https://platform.example.invalid",
  "endpoint": "https://api.day.app/push",
  "device_key": "REPLACE_IN_PRIVATE_HOST_CONFIGURATION",
  "database": "postgresql://notifier:<PASSWORD>@db.example.invalid/app?sslmode=verify-full",
  "psql_program": "/usr/bin/psql",
  "state_directory": "/var/lib/codexsymphony-notifier",
  "local_fixture": false
}
```

`application_origin` 必须精确匹配平台登录的公开 origin。通知器接受 PostgreSQL URI 的 sslmode、sslrootcert、options 参数，清空继承的数据库/代理环境并强制只读事务和 5 秒 SQL 超时；psql 使用绝对路径，连接凭据仅进入该受限子进程环境，不进入参数。`local_fixture:true` 只允许 `http://127.0.0.1:<port>/push`，仅用于合成 key 的测试，不得用于生产。

将 `deploy/m2/codexsymphony-bark.service` / `.timer` 安装到 systemd 后，先以该用户在上述 namespace 运行一次，检查配置/权限、投影读取及 ledger。最后才启用 timer（开机后 30 秒、上次完成后 15 秒）。模板不会随主服务自动启用。缺项/文件权限错误均以非零状态和固定脱敏错误退出；timer 不会增加任一业务或模型重试预算。

## 状态与上线验收

宿主以通知器用户执行：

```sh
python3 -I /opt/codexsymphony-notifier/bark.py --config /etc/codexsymphony-notifier/bark.json --status
```

输出只含任务 ID、kind、哈希 identity、时间、次数和状态。`accepted` = Bark 接受；`pending` = 等待有限重试；`sending/unknown` = 曾开始投递、结果可能未知；`failed` = 拒绝/次数或 deadline 耗尽；`obsolete` = 已处理或变化。失败不清除待办。私有 service journal 的固定错误表示配置、读取或本地状态不可用，应先修复宿主条件；不自动重置失败记录或无限重发。手机直接打开应用待办箱即可继续，通知器不提供业务恢复按钮。

真实上线须记录：准确发布 SHA、独立 UID/挂载和网络 namespace、上述数据库 ACL、实际 CA/DNS/出口、Bark 安装与设备授权、同源登录返回原任务、旧通知重复点击无动作、手机离线后待办仍可处理。用明确授权的测试任务向真实手机发送一次，保留脱敏结果，不保存 key/完整请求。该步骤属于最终 M2 部署集成，当前受控 fixture 不能替代真实手机/公网验证。

## 本地验收

```sh
python3 /opt/symphony-env/run.py python3 -m unittest tools.tests.test_bark tools.tests.test_bark_network tools.tests.test_m2_executor -v
python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py
```

第一组使用合成 key、实际本地 HTTP 接收器、持久 SQLite、独立 PostgreSQL schema 和独立进程，覆盖 JSON POST、错误/超时、有限重试、重启/并发去重、版本变化、无业务写权限与泄漏检查；网络 namespace fixture 实际证明 Agent 所在网络不能连接通知器 namespace 的本地接收器。编码和验证实际执行器均不能读取 notifier sentinel。第二组需先 Cargo / Angular build，覆盖桌面/手机登录返回、安全页面、持久待办、旧版本拒绝。宿主旧 `e2e.py` 固定 HTTP origin，与 M2 HTTPS 登录不兼容；使用仓库已有 HTTPS fixture，不放宽认证。

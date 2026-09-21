# 平台账号与会话（GH-71）

本增量交付平台认证、登录页和现有客户端配套。所有业务路由（包括未知路径和新增路由）由最外层认证中间件保护；Requirement、Run、GitHub/CI 和验证证据的事实边界保持独立。没有公开注册、默认账号或生产测试绕过。

## 宿主配置与账号管理

服务继续只监听 loopback。必须显式提供 `WEB_ORIGIN=https://实际入口` 和 `AUTH_CONFIG` 文件路径，文件内容例如：

```json
{"public_origin":"https://example.test","trusted_proxies":["127.0.0.1"]}
```

两处 origin 必须精确相同，不允许路径、用户信息、查询、片段或 HTTP。`trusted_proxies` 只接受明确的单个 IP；不支持通配网段。入口反向代理须覆盖（不能追加）`X-Forwarded-Proto: https`、`X-Forwarded-Host: 实际入口authority` 和单个 IP 的 `X-Forwarded-For`，并将上游 Host 改为实际 loopback 监听地址。受信代理缺头、多值或配置不匹配返回 403；其他来源的转发头不参与身份或限流键计算。Cloudflare Access 头从不授予身份。共享 loopback 的不可信进程不属于可安全受信的代理，M2-2 须验收宿主隔离和真实入口。

使用该服务二进制的 `auth init --stdin-json` 初始化账号，`auth change --stdin-json` 或 `auth reset --stdin-json` 修改／重置密码。操作使用宿主受限 `DATABASE_URL`，不能由 Web 客户端调用。输入格式是 `{"username":"用户名","password":"密码"}`，仅接受匿名管道或权限不含 group/other 位的普通 stdin 文件；拒绝终端回显、设备输入和密码参数。输入最多 8192 字节；用户名 3–128 个 ASCII 字母、数字、点、下划线、短横线，密码 12–1024 字节。不要使用包含密码字面量的 shell 命令。

可由宿主管理员使用安全终端读取，再通过匿名管道交给二进制（下面不包含真实密码）：

```python
import getpass, json, os, subprocess
subprocess.run([os.environ['CODEXSYMPHONY_BINARY'], 'auth', 'init', '--stdin-json'],
    input=json.dumps({'username': input('Username: '),
                      'password': getpass.getpass('Password: ')}).encode(), check=True)
```

数据库仅保存每账号独立随机盐的 Argon2id PHC 哈希。改密与全部会话撤销在同一数据库事务完成，并与并发登录按账号行锁串行化。迁移 0022 只新增认证表，不改写 M1 草稿、队列、授权、预算或已保存工作。

## 会话、请求与客户端行为

`__Host-codexsession` 是 256 位随机不透明 Cookie，带 `HttpOnly; Secure; SameSite=Lax; Path=/`，不设 Domain。数据库仅保存 SHA-256 摘要、账号、绝对到期时间与撤销标记。登录会话绝对 8 小时，不滑动续期；匿名 CSRF 启动会话 10 分钟。登录旋转 Cookie 并撤销启动会话；退出立即撤销当前会话。重启不改变期限或撤销状态。可注入的时钟仅为 Rust 测试依赖，没有环境变量、HTTP 参数或生产配置绕过。

公开端点只有 GET `/api/health`、GET `/api/auth/csrf`、POST `/api/auth/login`。健康响应不包含业务数据。登录前先取 CSRF 启动证明；登录、退出和全部写请求必须具有精确 Origin、有效 Cookie 及与该 Cookie 绑定的响应派生证明。旧固定 `x-codexsymphony-csrf: 1` 无效；单有 Cookie 或 Origin 也无效。返回证明的成功响应禁止缓存。

每 15 分钟固定窗口按账号最多 10 次、可信来源最多 50 次登录尝试，两维计数均持久化并原子递增，成功尝试也计数；未知账号同样执行密码 KDF。凭据错误统一为“用户名或密码错误”，达到限额为相同提示的 429。错误、撤销和期限检查都发生在业务处理之前。

登录页支持密码管理器与粘贴；令牌和 CSRF 证明不写 localStorage。路由守卫恢复后端会话，401 后保留当前路由组件的未提交输入于页面内存、隐藏业务内容并显示重新登录入口；刷新／关闭页面不会保留尚未保存到后端的输入。登录成功后用户必须检查并手动重新提交，客户端从不重放写请求。返回地址仅允许规范站内路径及受限查询字符，拒绝外站、协议相对、反斜线、编码分隔符与重复编码。退出亦保留当前未提交输入，登录后可继续。

## 验收入口与边界

全部命令在分配工作区和一次性测试数据库执行：

- `python3 /opt/symphony-env/run.py cargo test --workspace --locked`：真实账号登录、Cookie/CSRF、可控时钟 8 小时期限、密码撤销、持久限流、代理与身份头拒绝、实际路由声明发现及业务表前后快照、M1 回归与真实 Runtime。
- `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets --locked -- -D warnings`。
- `web/angular` 内 `npm test -- --watch=false`、`npm run lint`、`npm run build`。
- `python3 /opt/symphony-env/run.py python3 tools/auth_session_acceptance.py`：受控验证证书的 HTTPS、真实进程重启、旧会话／旧密码拒绝、改密及重置、跨重启双维限流、凭据日志扫描。
- `python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py`：先构建 Rust 二进制和前端产物；专用 schema、临时测试证书、仅信任该证书公钥的 Chromium，以及实际 CookieJar 登录。运行 Desktop Chrome／Pixel 7 的既有业务与新增认证用例、axe、草稿恢复和共享数据。不使用关闭全部证书检查的选项，不改宿主信任库。
- `python3 /opt/symphony-env/run.py python3 tools/auth_contract_acceptance.py`：已安装可信采集器的真实 HTTPS 场景；凭据只在内存中验证，保留脱敏且再次校验的观测。此结果不是签名 Gate 证据。

AAuth01–04/06 与 AAuth05 的身份头拒绝部分由以上测试联合覆盖；实际通过情况、源码身份及日志在本次交付 evidence manifest 记录。真实公网 HTTPS、源站隔离和生产配置属于 M2-2／最终集成，不以本次受控测试替代，也不宣称 M2 已远程上线。完整精确提交双 Gate 由独立宿主执行。

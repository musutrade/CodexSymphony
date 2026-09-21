# M2 HTTPS 与执行凭据边界

本项交付可安装配置、检查工具与隔离 fixture。公网部署需最后集成项记录真实域名、证书、Tunnel 授权及验收；旧 `factory.aglmud.org` 的 502 或移除 Access 记录不是本次证据。这里没有创建账号密码、真实密钥、Tunnel 或修改生产数据库。

## 安装前提与角色

Linux 需要启用非特权 user/PID/mount namespace、安装 bubblewrap、Python 3、Nginx、受支持的固定 Codex/工具链及 PostgreSQL。命令均由管理员在目标宿主执行；工作区只运行下述一次性 fixture。不要用关闭 TLS 验证、特权容器、共享宿主 HOME 或复制生产密钥来修复预检。

| 角色 | 文件与动作 |
|---|---|
| 控制面/GitHub Broker | 私有 DB 配置、状态与 GitHub App 凭据；仅经既有授权/候选归属检查执行远端动作。代码命令交给部署执行入口，不能在持有秘密的文件系统视图中直接运行候选脚本 |
| 编码 Run | 管理员固定 launcher，独立 PID/mount 视图，只挂载当前 worktree、独立 codex-home、项目工具与获授权测试资源；普通 Shell/Git/编译继续可用 |
| 开发验证 | 同一个部署入口的 validation 配置；没有 codex-home，只挂载精确候选 checkout 与项目测试资料；原 `validation_runner` 保留输入身份、失败输出和未知结果闸门 |
| 正式验证 | 仓库外已安装独立 verifier 获取发布的精确 SHA，按批准策略采集并签名；不使用编码环境的测试输出作为签名。签名私钥、批准文件与 nonce 台账不挂进任何候选测试视图 |
| HTTPS/Tunnel | 独立入口服务持有 TLS/Tunnel 文件；只转发 HTTP，不授予平台身份 |

这是本人可信项目的部署边界，不承诺防御任意恶意代码、多租户或内核漏洞。`executor.py` 使用真实挂载与 PID namespace，而非只过滤环境变量。网络仍是获授权项目的普通开发网络；管理员必须确保环境中只有项目测试凭据，数据库/远端服务自身也检查身份。平台数据库 URL、GitHub token、签名秘密、代理密码不得写入 executor 配置或工具目录。

## 管理员安装与检查

1. 从已通过必需检查的精确版本安装：`sudo sh tools/deployment/install.sh RELEASE_ID`。脚本只在 `/opt/codexsymphony-m2/releases/RELEASE_ID` 暂存不可覆盖的版本，不切换服务。保存输出摘要及产品二进制 SHA。部署程序、配置父目录由管理员拥有，候选不得改写。
2. 创建控制面、GitHub、verifier 独立秘密目录（示例分别为 `/etc/codexsymphony-control`、`/etc/codexsymphony-github`、`/etc/codexsymphony-verifier`），目录 0700、凭据文件 0600，服务账号只获所需读取权限。GitHub 与签名私钥在对应服务内生成/装载，绝不复制进源码或执行工作区。测试配置只放获授权项目测试凭据；日志关闭请求体、Cookie、Authorization、密码、CSRF 和 URL 中凭据的输出。
3. 从 `deploy/m2/*-boundary.json` 创建私有 0600 配置。替换为实际 canonical 路径（包括解析 `/etc/resolv.conf` 的 symlink）；缺路径或非 canonical 路径会拒绝启动。列全平台状态、凭据、Broker、签名目录到 `private_paths`；任何挂载与它们交叠均拒绝。不要挂载整个 `/home`、`/etc`、`/tmp` 或执行状态根。准备独立、无秘密的工具树。普通 Git worktree 如需共同 Git object/ref 目录，显式增加该项目的 Git 存储挂载，不挂控制面状态；保持 GitBroker 的候选与恢复归属检查。
4. 将 Runtime 的 `preparation.launcher` 配为 `["/opt/codexsymphony-m2/releases/RELEASE_ID/coding"]`；它的固定 program 为锁定 Codex 二进制。`runtime_home_root` 对应产品 execution 根，当前 `CODEX_HOME` 必须是其下的 `codex-home`。preparation adapter 的初始化/探测与正式启动使用相同 launcher，探测脚本也需放在批准的只读工具树。不要绕过现有 Runtime 预检。
5. 开发验证 `Plan.entry` 指向该版本的 `validation`，`entry_sha256` 为实际文件 SHA-256，固定 program 为管理员安装的项目检查入口；工作目录由既有精确候选恢复提供。为每个项目准备完整配置，未准备时保持 Runtime/验证未启用。模板本身不是可用部署声明。正式验收仍用独立 verifier，不能用此开发 Plan 冒充签名来源。
6. 按 [平台账号说明](platform-authentication.md) 的匿名 stdin 管道初始化账号；无默认密码、无公开注册。用同一方法改密/重置并验证旧会话撤销。保持 M1 数据与数据库迁移备份，失败不重建生产库。

## HTTPS、受信代理与源站

复制并修改 `deploy/m2/nginx.conf`、`auth.json` 与 `cloudflared.yml` 到管理员管理的配置目录。模板的 `platform.example.invalid` 必须全部替换成已授权域名。Nginx TLS 证书必须包含该域名 SAN；私钥只由入口服务读取。先执行 `nginx -t -c /实际路径/nginx.conf`、`cloudflared tunnel --config /实际路径/cloudflared.yml ingress validate`，证书/配置失败不能启用。

产品 `BIND_ADDRESS=127.0.0.1:3081`，`WEB_ORIGIN=https://实际域名`，`AUTH_CONFIG` 指向同 Origin 的私有配置，只信任实际代理来源 `127.0.0.1`。API 使用 socket peer 判断代理，不根据 Access、邮箱、CF-Connecting-IP 或 Forwarded 判断用户。Nginx 固定后端 Host 和 HTTPS/外部 Host，覆盖来源头、删除 Access 身份头。Tunnel 场景以本地来源合并限流，不能直接相信客户端提供的 CF IP 来扩展额度。

Nginx 只监听 `127.0.0.1:8443`，Tunnel 指向这个 TLS 入口，验证 originServerName，禁止 `noTLSVerify`。Tunnel 最后一项为 404；不得另配直达业务源站的入口。操作已有 Tunnel/DNS 需最后集成项的真实授权接口，本 Issue 不自动创建或修改。采用直连 HTTPS 时，管理员只开放代理的 443，源站仍为 loopback，并重新记录监听/防火墙检查。

入口启动后，在宿主执行以下只读检查，PID 必须取自本次启动的控制面服务：

```sh
python3 tools/deployment/check_ingress.py --origin https://实际域名 --pid 控制面PID --backend 127.0.0.1:3081
```

检查器验证 TLS 信任链、匿名/伪造及 JWT 格式 Access 头均收到 401、该 PID 无额外/非 loopback TCP listener、直达源站不返回业务数据。在另一台机器再运行仅含 `--origin` 的 HTTPS 检查；另用 `nc -vz -w 5 实际源站IP 3081` 确认连接失败（HTTPS 检查器本身不探测公网源站端口），并实际完成电脑/手机账号登录、Secure Cookie、退出/重置后的旧会话拒绝。真实 Access token 不应进入日志；JWT 格式 fixture 不是 Cloudflare 签发 token 验证。

## 切换与回退

记录现有二进制、入口配置和 Runtime/Plan 路径摘要，先暂停领取，等待既有 supervisor 匹配身份的静止回执。存在遗留进程、旧 incarnation/启动 tick 不匹配或未知验证结果时保留阻塞，不能删除状态文件后重跑。先完成新版本离线配置检查、fixture 与正式精确版本 Gate，再由宿主授权的部署操作切换产品/代理配置路径并重启，重跑上述检查及真实登录。新入口失败则恢复旧版本二进制/配置路径与监听，重新核对进程及身份；不删除迁移表、草稿、授权、用量、已保存工作或验证失败证据。密码已重置时不回滚账号数据库以恢复旧密码。

原独立 verifier 安装与批准流程见 [remote-gate](remote-gate.md)。它重新取精确提交，对可信配置及 collector 签名来源进行验证，并以固定 App 身份发布结果；同名不同来源检查、旧 SHA、签名篡改或缺少证据均不得当成通过。生产切换必须保留该版本正式结果与来源；开发 fixture 成功不替代该记录。

## 本项可运行验收

```sh
python3 /opt/symphony-env/run.py cargo test --workspace --locked
python3 -m unittest discover -s tools/tests -p test_m2_executor.py -v
python3 /opt/symphony-env/run.py python3 tools/m2_https_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/auth_session_acceptance.py
python3 -m unittest discover -s tools/tests -p test_gate_host.py -v
python3 tools/gate.py config check
```

首个命令包含真实 validation_runner namespace 测试、Runtime、停止/遗留进程/身份不匹配、重启恢复及 GitHub 检查来源回归；进程测试使用无敏感 sentinel，项目源码/测试资料可读、平台/GitHub/签名资料不可读。HTTPS fixture 实际启动本仓库 Nginx 模板（仅替换临时路径、loopback 端口与 fixture 主机名），使用专用数据库 schema、临时证书与独立账号，客户端只信任该证书，退出时删除测试 schema；报告写入 `artifacts/gh72/https.json`，保留成功/失败阶段且不输出密码或 Cookie。完整精确版本双 Gate 由宿主在发布后执行。生产域名、Tunnel、证书运维、真实签名来源及跨设备检查属于最后集成记录的待验项，不能由此 fixture 自动勾选。

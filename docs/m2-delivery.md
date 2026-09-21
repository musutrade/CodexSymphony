# M2 远程接续交付与上线清单（GH-76）

本记录区分代码交付、受控集成和实际部署。受控测试不代表公网可用；M1 本机服务健康不证明 M2 上线。生产数据库、Tunnel、真实手机 Bark 和离机存储未在本工作区操作。

## 版本与验收范围

开始版本为 main `86f19bacad6c36d025719a61ab9cc46c55e01c77`，包含 #71–#75 的最终 PR [#77](https://github.com/musutrade/CodexSymphony/pull/77)、[#78](https://github.com/musutrade/CodexSymphony/pull/78)、[#79](https://github.com/musutrade/CodexSymphony/pull/79)、[#80](https://github.com/musutrade/CodexSymphony/pull/80)、[#81](https://github.com/musutrade/CodexSymphony/pull/81)。已重读最终实现及认证、部署、手机、通知和恢复手册；前置 PR 的通过记录只证明依赖交付，不作为本次测试结果。

本次重新构建完整前后端，使用同一产品源码与二进制。GH-76 补齐认证/恢复测试的证据目录创建，并增加真实会话到期、重启后的拒绝检查和服务/HTTPS 入口进程身份记录；没有改变产品认证、调度或门禁策略。测试源码的最终清单及发布树另行绑定，不能把本机 baseline SHA 当作 GitHub 最终 PR head。

可读结果与逐项原始证据校验值见 [GH-76 验收记录](quality/gh76/README.md)。工作区 `.symphony-evidence.json` 绑定原始日志、数据库事实、截图及文件清单；是否已归档以宿主 after_run 回执为准。正式 Harness-Gate / Trusted Harness-Gate 由独立服务对最终 PR head 重新运行，本地测试不产生签名验收。

## 验收矩阵

`passed` 只适用于“受控集成”列明确列出的边界。`blocked` 表示仍需部署输入/现场证据，不能换成 N/A；`out-of-M2` 保留后续里程碑 AC，不表示已完成。

| 条目 | 受控集成 | 持久事实与副作用检查 | 真实部署 |
|---|---|---|---|
| AAuth01 | passed | 桌面/Pixel 7 同账号读取同一草稿；真实管理员初始化、后端登录；全部业务路由匿名 401，业务表前后不变 | blocked：实际电脑/手机同源登录 |
| AAuth02 | passed | 错误账号/密码统一响应，账号/来源双维限流跨进程重启，伪造转发头不能扩展额度 | blocked：实际可信代理链与限流复验 |
| AAuth03 | passed | Argon2id 独立盐、会话摘要、Secure/HttpOnly/Lax；精确八小时测试、真实到期拒绝；退出/改密/重置撤销跨重启 | blocked：现场双设备撤销 |
| AAuth04 | passed | 登录/业务写的 Origin 与会话绑定 CSRF 负例，合法写入；无认证副作用；日志哨兵扫描、浏览器无令牌持久存储 | blocked：实际入口日志和代理配置 |
| AAuth05 | passed | 交付的 Nginx TLS 模板、受信证书、PID loopback listener、源站匿名拒绝；无 Access 登录及伪造/JWT 格式身份头拒绝 | blocked：公网证书、外部源站探测及真实 Access 头；未获取真实 token |
| AAuth06 | passed | 401 后重新登录，恢复草稿而不自动重放写请求；外站/编码返回地址拒绝；管理员重置后旧凭据拒绝 | blocked：实体手机过期重登 |
| B02 | passed | 手机创建/导入、保存重读、整组评审与同一队列；草稿无执行授权 | blocked：实体手机现场流程 |
| B03 | passed | 关闭桌面后问题/旧 Run 超时，真实停止保全；重启后手机回答，新 Run 读取原工作与原问题版本，预算/授权保持 | blocked：部署 Runtime、真实手机现场接续 |
| B07 的 M2 部分 | passed | 队列依赖/版本冲突/差异评审；手机暂停后重启不续跑，显式恢复及取消保全；取消不伪造 Done 或满足依赖 | blocked：目标部署停止身份及恢复核验 |
| B08 的 M2 部分 | passed | 通知 HTTP 错误/超时、有限重试、持久去重、只读 DB ACL；离线重连待办仍在；真实编码/验证秘密哨兵读取失败、通知网络 namespace 不可达 | blocked：真实 Bark 手机、生产权限和出口 |
| 日常恢复 | passed | PG16 custom dump/restore、AES-GCM、mTLS 上传回读；全表/序列与 Git/通知台账比较；撤销/到期会话不复活，恢复 guard 禁外部写及普通启动 | blocked：真实离机目标、writer units、独立恢复环境及切换演练 |
| B01、B04–06 | out-of-M2 | M1 已有草稿/预算等行为的回归不等于 M3 完整流程；未实现 validation_only 执行闭环、CI 修复或自动合并 | out-of-M2 |
| B08 的 CI/自动合并部分 | out-of-M2 | 既有观察/交付回归不代表 PR/head/merged SHA 全自动验收完成 | out-of-M2 |

手机为真实 Chromium 的移动视口，不是实体设备。隔夜场景只推进一次性数据库中的 fixture 时间；使用脚本 app-server 对端与合成 GitHub 能力观察以确定性触发问题，实际执行服务、数据库、Git、子进程和浏览器。全量 Rust 测试另运行 pinned Codex 的 `runtime_real`（本地模型协议接收器，不调用真实模型）。通知使用合成 key 和真实本地接收器；备份 mTLS 对象服务位于受控本机，不能记作真正离机。

## 可重复运行

先按 `.agent-env/README.md` 检查工具和一次性数据库。只使用分配的测试资源；全部启动产品进程的命令必须串行，不能争抢固定实例锁。临时目录在工作区内，浏览器只信任测试证书，不关闭全局 TLS 验证。

```sh
mkdir -p .agent-tmp artifacts/gh76
python3 /opt/symphony-env/run.py cargo build --workspace --locked
(cd web/angular && npm ci --offline --no-audit --no-fund && npm run build)
# 提供环境的 /tmp 映射到工作区 .agent-tmp；Unix socket 必须使用短路径。
# 若上次套件中断，先保留失败事实，再只重建 disposable test fixture。
# python3 /opt/symphony-env/dbctl.py recreate test
TMPDIR=/tmp python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 /opt/symphony-env/run.py cargo check --workspace --all-targets --locked
(cd web/angular && npm run lint && npm test -- --watch=false)
export TMPDIR="$PWD/.agent-tmp"
python3 /opt/symphony-env/run.py python3 tools/m2_https_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/auth_session_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/mobile_recovery_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/auth_contract_acceptance.py
python3 /opt/symphony-env/run.py python3 -m unittest tools.tests.test_bark tools.tests.test_bark_network tools.tests.test_m2_executor tools.tests.test_daily_recovery -v
python3 -m unittest tools.tests.test_gate_host -v
python3 tools/gate.py config check
```

完整质量门禁沿用 [.harness-gate/QUALITY.md](../.harness-gate/QUALITY.md) 和 [remote-gate](remote-gate.md)，包括源码测量、CRAP ≤ 10、秘密扫描、架构检查和七个必需步骤。缺测量或不支持的测量仍阻断；不得修改策略或用前置版本的签名替代当前版本。

## 生产操作前必须补齐的输入

| 输入/责任 | 可以开始下一步的条件 |
|---|---|
| 发布与操作授权 | 指定 release SHA、二进制/前端摘要、精确 SHA 的两个受保护检查通过；宿主登记该 revision/可执行文件/操作的部署 grant |
| 账号与控制面 | 管理员安全终端、私有 AUTH_CONFIG/DB 配置；无默认密码；控制面/GitHub/verifier 凭据独立托管 |
| HTTPS | 授权域名与证书/续期责任、Tunnel ID 和凭据保管、真实可信代理来源、源站监听/防火墙；不复用历史 502 作为验收 |
| 执行边界 | 管理员固定 coding/validation launcher、canonical 路径与独立工具树、私有目录清单、UID/namespace/cgroup、工作与归档挂载身份及配额 |
| Bark | 授权设备、受限 key 配置、通知器独立 UID/网络 namespace、只读 projection 角色、CA/DNS/批准出口；`accepted` 不等于手机已收到 |
| 备份恢复 | 完整 writer units、停写窗口、受限 AES/mTLS key custody、加密离机目标 ACL/容量/保留、独立空恢复库/目录与外部出口拒绝 |
| 验收与回退 | 现场双设备、授权测试任务、旧 release 与升级前备份、迁移兼容性证据或隔离恢复路线、截点后事实对账责任人 |

没有这些输入时维持关闭/拒绝或现有服务状态；本 Issue 不请求把真实秘密发进工作区。部署操作尚无精确 grant，因此仅交付以下管理员执行步骤。

## 最后部署步骤与停止条件

1. 在宿主取得经过双 Gate 的精确 release，核对源码和产物摘要。按 [日常恢复](daily-recovery.md) 记录当前迁移版本/checksum、旧二进制和配置；暂停新领取，等待原执行组停止及工作保全证明。停止并 mask 完整 writer units，核对 cgroup 无后代进程。未静止、身份不符或无法备份时停止升级。
2. 使用受限配置运行已安装恢复工具：`python3 /opt/symphony-recovery/backup.py --config /etc/symphony-backup/config.json backup`，对返回的归档执行同入口 `verify --archive <绝对归档路径>`。离机回读摘要、隔离 `restore --archive <绝对归档路径> --target <受限目标配置>` 及两次 `--recovery-drill` 核验均通过后才继续。任何 `not_configured`、校验失败或资料遗漏均不能批准远程上线。
3. 在目标宿主从该 release 执行 `sudo sh tools/deployment/install.sh RELEASE_ID`，暂存后核对输出摘要。按 [部署手册](m2-deployment.md) 安装私有边界配置、Runtime/Plan launcher 和只允许必要目录的挂载。用无敏感哨兵在实际 coding/validation 两条路径做读取否定测试，验证 supervisor 停止后代和拒绝旧身份；配置检查本身不能替代执行证明。
4. 在隔离副本验证新迁移及事实保持，再于已授权停写窗口切换控制面二进制/配置。账号由 [认证手册](platform-authentication.md) 的安全终端和 stdin 命令初始化/重置，密码不得放命令参数、环境或日志。启动唯一控制面，核对 PID、启动身份、迁移 checksum、`/api/health` 与冷启动屏障。保留暂停项，禁止为了健康检查清空数据库或重置预算。
5. 替换 [Nginx/Tunnel 模板](../deploy/m2/) 的域名/路径；运行 `nginx -t -c <实际配置>` 和 `cloudflared tunnel --config <实际配置> ingress validate`。Tunnel 只通向验证证书的 TLS 代理，末路由 404；源站只在 loopback。通过授权接口切换入口后，在宿主运行 `python3 tools/deployment/check_ingress.py --origin <实际HTTPS-origin> --pid <本次控制面PID> --backend 127.0.0.1:3081`，外部机器运行仅含 `--origin` 的检查并确认源站 3081 不可达。TLS、代理伪造、源站或匿名边界任一失败，立即关闭新入口并保持任务暂停。
6. 实体电脑/手机同账号验收登录、绝对到期/退出/改密撤销、草稿及整组授权、队列冲突、关闭电脑后的后台和隔夜接续、暂停重启/取消。记录任务/Run/问题版本、停止/恢复证据和消耗，不能只留截图。失败保留原工作和待办，暂停切换，不将 PR/CI/Gate 状态写成业务 Done。
7. 按 [Bark 手册](bark-notifications.md) 配置独立用户/namespace/数据库 ACL，再在受限配置写入真实 key。先 `python3 -I /opt/codexsymphony-notifier/bark.py --config /etc/codexsymphony-notifier/bark.json --status`，核验后仅对授权测试任务发送一次；手机打开通知需登录并返回原任务。复验失败/离线仍有待办和重复点击不执行旧决策，最后才启用 timer。失败则停通知器、保留台账，不重置 attempts/deadline 或业务预算。
8. 记录上述现场结果、SHA/摘要和真实离机回读/恢复证据后，才可宣布 M2 远程上线并按原业务授权恢复领取。M3 条目继续保持 out-of-M2。

回退必须先阻断新入口/领取并保全事实。仅配置故障且数据库兼容性已证明时，恢复旧配置/二进制再验身份与健康；迁移失败、兼容性未知或数据校验失败时，保持停写，在新隔离目标恢复升级前包，不让旧二进制直读已迁移生产库。按恢复手册对账截点后的授权、用量、取消和保存工作，撤销恢复库全部 session、保留限流并设置全局暂停，经管理员批准才解除 recovery guard 和切换。不得删除迁移记录、复活撤销会话或用恢复备份抹去截点后事实。

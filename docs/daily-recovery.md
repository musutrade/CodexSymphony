# 日常备份、升级保护与隔离恢复（GH-75）

此工具覆盖 M1/M2 的 PostgreSQL `public` 应用数据、execution（包含 Git common directory、worktree、index、未提交和未跟踪文件、运行/保存证据）、cold 归档、配置恢复资料及 Bark SQLite 台账。备份、校验和恢复是宿主管理员操作，不向 Agent 暴露凭据。默认配置 `deploy/m2/backup.json` 关闭；没有离机目标时明确报告 `not_configured`。本项受控测试不是生产 M2 远程上线或真实离机灾备验收。

## 一致性边界

当前只交付**停机备份**。管理员先停止并 runtime-mask 控制面、Bark timer/service、编码和验证 executor 单元；停止操作须已确认整个 cgroup 无后代进程。部署必须使用 `KillMode=control-group`，禁止进程逃出受管 cgroup；外部维护脚本、Git 写入、数据库迁移也必须停止。配置列举完整写入单元，漏列写者不能获得一致性保证。工具检查单元 inactive/masked 和 cgroup，持有产品固定 instance lock 与通知器 `worker.lock`，阻止它们启动；再对全部应用表持有 SHARE 锁（阻止业务写和表结构修改），导出同一 PostgreSQL snapshot 供 `pg_dump` 使用。在这些锁持有期间打包全部资料，并前后复核文件清单及所有表、序列、通知投影摘要。未知/缺失资料、活动锁、挂载身份变化或内容变化均拒绝成功。

这不是“在线 pg_dump 等于工作目录一致”。运行中只做数据库逻辑快照不能用于本恢复流程。若未来使用存储层在线快照，必须另外证明数据库与所有工作、cold、配置、通知台账同时冻结的边界；本工具不接受一个人工 `consistent=true` 标记替代证明。数据库角色、扩展安装、TLS/密钥 custody 属于独立恢复引用，不能靠应用表恢复自动重建。外部表/额外 schema 须先纳入经过评审的恢复范围，不能静默遗漏。

## 管理员安装与配置

在 Agent/候选验证不可读的宿主管理目录安装 `apps/recovery/*.py`，Python 3.12+、`requirements.txt` 锁定的 cryptography、PostgreSQL 16 的 psql/pg_dump/pg_restore。所有脚本、二进制须来自已审定提交；记录 Git SHA、服务二进制 SHA-256 和必需检查结果，不能把 Gate/CI 成功当成业务 Done。配置、32 字节随机 AES 密钥、mTLS 私钥及临时/备份目录必须为操作 UID 所有，文件 0600、目录 0700，父路径无软链接。服务二进制也按 0700 安装。管理员应将工作盘、临时空间及备份盘配额纳入现有容量管理，预留明文 tar + DB dump + 密文的峰值空间（至少最大包三倍），不能使用正在被备份的目录存储备份。

在受限编辑器中配置以下 JSON；示意内容不包含可用密码。`database` URI 只存于受限配置文件，通过进程环境传递给 libpq，绝不放在命令参数或日志中。支持 sslmode/sslrootcert/sslcert/sslkey；跨主机 PostgreSQL 应使用 verify-full。不要使用 `set -x`、打印环境或将配置提交仓库。

```json
{
  "enabled": true,
  "fixture": false,
  "database": "<管理员注入的数据库 URI>",
  "controller_lock": "/tmp/codexsymphony-controller.lock",
  "notifier_lock": "/srv/symphony/notifier/worker.lock",
  "units": ["codexsymphony.service", "codexsymphony-bark.service", "codexsymphony-bark.timer", "symphony-coding.slice", "symphony-validation.slice"],
  "roots": {
    "execution": {"path": "/srv/symphony/execution", "identity": [123, 456]},
    "cold": {"path": "/srv/symphony/cold", "identity": [123, 457]},
    "configuration": {"path": "/srv/symphony/recovery-config", "identity": [123, 458]},
    "notifier": {"path": "/srv/symphony/notifier", "identity": [123, 459]}
  },
  "destination": "/srv/symphony-backup",
  "key_file": "/etc/symphony-backup/encryption.key",
  "recovery_references": "/etc/symphony-backup/custody.json",
  "source_sha": "<已验源码完整 40 位 SHA>",
  "binary": "/opt/symphony/bin/codexsymphony-server",
  "binary_sha256": "<已验二进制完整 SHA-256>",
  "retain_count": 7,
  "max_bytes": 4294967296,
  "offsite": null
}
```

目录 `identity` 是管理员确认挂载后用 `stat` 读取的设备号/inode，不得复制示例值；更换挂载时重新盘点资料。`configuration` 包含部署配置、依赖版本、存储映射、恢复步骤与恢复引用，**不包含备份密钥本身**。`custody.json` 必须提供 database/auth/github/signing/bark/backup_key 六个非敏感保管位置/恢复步骤引用（包括账号密码哈希随 DB 恢复、签名私钥不随 Agent 备份）。真正的密钥在独立管理员 custody 中轮换与恢复。丢失加密密钥不能恢复包；新密钥不能解旧包。

离机目标配置为：`{"url":"https://backup.example.invalid/symphony","ca":"/etc/.../ca.pem","certificate":"/etc/.../client.pem","private_key":"/etc/.../client.key","fixture":false,"max_object_bytes":4294967296}`。使用专用 mTLS 身份，仅准许该前缀的 PUT/GET/DELETE，服务端目录受限并设置容量配额。URL 禁止凭据和重定向。上传的只有 AES-256-GCM 密文；每包独立随机 nonce，完整性认证覆盖格式头与密文；下载回读 SHA-256 一致后才报告 `offsite_https_verified`。证书信任和“确实离机”由管理员确认，不能把同机挂载冒充离机。真实 fixture 始终报告 `fixture_https_verified`。

```sh
python3 /opt/symphony-recovery/backup.py --config /etc/symphony-backup/config.json status
python3 /opt/symphony-recovery/backup.py --config /etc/symphony-backup/config.json backup
python3 /opt/symphony-recovery/backup.py --config /etc/symphony-backup/config.json verify --archive /srv/symphony-backup/backup-....enc
```

成功回执为每包 `.receipt.json`，只含密文摘要和离机状态。失败上传保留本地包并报告失败，不能把它当作成功离机副本。只在新包完成后按 `retain_count` 清理带正确回执的旧包和对应远端对象；摘要不符、远端删除失败、目标变更时停止自动清理并要求管理员对账。容量不足拒绝新备份，保留之前副本。断电遗留 `.backup-*`、无回执密文及未知远端对象由管理员核验后清理，不自动删除唯一副本。远端应另外配置按此保留窗口的生命周期/配额，处理上传进程被杀后无法落本地回执的孤立对象。备份目标禁用 shell/执行权限。

## 隔离恢复与重启核验

1. 在独立机器或受控测试资源上创建**空的** `symphony_restore_<唯一标识>` PostgreSQL 16 数据库、空的 0700 目录。恢复账号不能访问生产数据库。不要把生产服务、Tunnel、Bark、真实模型/GitHub/签名凭据挂载到演练目标；网络策略只允许访问隔离数据库和 loopback，外部动作出口拒绝。备份来源必须是管理员认可的可信数据库/包；`pg_restore` 会执行包内 SQL，不能用超级用户恢复未知来源 SQL。
2. 受限 target JSON 为 `{"database":"<隔离目标 URI>","directory":"/srv/symphony-drill/<唯一标识>"}`。运行：

```sh
python3 /opt/symphony-recovery/backup.py --config /etc/symphony-backup/config.json restore --archive /srv/symphony-backup/backup-....enc --target /etc/symphony-backup/drill.json
```

3. 工具先写入持久 `public.symphony_recovery_guard`，再验证 AES-GCM、目录/文件摘要并在单事务中 pg_restore。普通服务启动在迁移和恢复 worker 前拒绝这个数据库。失败保留 guard，禁止自动清空或重复覆盖；管理员保留证据、销毁这个**隔离**目标后再新建目标重试。恢复不会修改生产数据库或原包。
4. 从受限服务环境注入隔离 `DATABASE_URL`、`BIND_ADDRESS=127.0.0.1:0`、`WEB_ORIGIN=https://localhost:4200`，启动同一已验二进制 `codexsymphony-server --recovery-drill`。只提供 `GET /api/recovery` 脱敏统计；数据库连接默认只读。不启动迁移、generation 恢复、调度、Runtime、GitHub、存储清理或通知器，不加载其配置，不暴露登录或业务写路由。停止并再次启动，核对结果及包内全部表/序列摘要不变。普通服务启动须仍失败。
5. 逐项比对包内 manifest：草稿父子、依赖、AC/覆盖、评审版本与授权快照；Requirement/组预算已用与预留；暂停/取消与收尾；未决问题与保存/恢复阶段；session 撤销/绝对到期、登录限流；通知待办 action_key、Bark attempts/deadline/去重台账；Git 暂存、未暂存、未跟踪和证据文件。完整事实摘要不打印业务正文或密码哈希。绝不从 Gate PASS、PR merged、CI success 推导业务 Done。

恢复资料放在 `materials/<原目录标签>`，manifest 保留原路径映射。演练不改写 DB 里的绝对路径、inode/PID 身份，也不运行任何保存资料；恢复到新硬件后必须通过产品原有身份/停止/存储恢复机制核验，不能手改 identity 宣称已通过。只读统计不是业务自动接续验收。

## 升级与生产回退顺序

1. 记录当前**确切源码 SHA、二进制摘要、schema 迁移版本/checksum、配置/协议版本、必需检查证据**；暂停新工作，等待运行及保存停止证明。暂停/取消是业务事实，不为升级清空授权或预算。
2. 按上文停写、备份、校验、离机回读，并完成隔离恢复。保留旧二进制与其配置和已验身份。记录备份截点后的任何业务事件窗口；未完成截点对账不能宣称无损恢复。
3. 对备份的隔离副本验证新版本迁移及恢复核验（迁移测试需要管理员受控环境；不可对生产演练）。正常生产升级由管理员在停写窗口安装已验二进制后运行既有 SQLx 迁移。失败保持停写，保存真实错误与 `_sqlx_migrations`，不要删除迁移记录、手动降级 schema 或重置 checksum。
4. 旧程序是否可直接读取已迁移库，必须有该旧/新源码和 schema 对的兼容性证明。本项没有一般性降级保证；没有证明时，只允许在新隔离目标恢复升级前包，用对应旧二进制核验，**不能旧程序直接连接已迁移生产库**。旧版本没有新恢复 guard 保护，必须由宿主网络/权限阻断外部副作用，不能因有 guard 表而假定旧程序安全。
5. 真正切换生产属于管理员恢复操作：确认唯一控制面、所有旧进程已停，完成截点后授权/用量/取消对账，验证目录映射/挂载身份和恢复引用，重新签发最小凭据。在开放认证入口前撤销恢复库的**全部** session（`UPDATE platform_session SET revoked=true`）；这处理备份截点以后发生的撤销，不能仅相信旧包的 revoked 值。保留限流记录；绝不到期续期或自动登录。
6. 在隔离库事务中设置全局 `execution_control.paused=true`，保留每项暂停/取消事实。只有确认目标、对账完成及备份可回退后，管理员才可移除 **recovery guard 表** 并切换连接，不能移除迁移记录。产品冷启动屏障和原有保存恢复校验继续生效；暂停项需用户明确恢复，取消项绝不复活。核验待办、草稿授权、预算和会话后才逐步恢复服务及通知，最后解除升级新增的全局暂停。无法证明截点后事实完整时继续隔离，不能报“零丢失”。

手机用户在维护窗口可能无法访问应用；管理员说明维护原因和预计恢复步骤。恢复后原待办/暂停入口仍是事实来源，旧通知不能执行过期决策。此次不新建另一个备份管理 UI。

## 受控证据与生产待配置项

复现（只在分配的测试资源）：

```sh
python3 /opt/symphony-env/run.py cargo build --workspace --locked
python3 /opt/symphony-env/run.py python3 -m unittest discover -s tools/tests -p test_daily_recovery.py -v
python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/gate.py config check
```

测试新建并销毁独立 fixture 数据库；真实 pg_dump/pg_restore、合成 AES 密钥、真实 mTLS HTTP 服务验证加密传输/校验、损坏/缺失资料/权限/容量/失败传输/锁竞争/重复恢复、两次服务启动和普通启动拒绝。`artifacts/gh75/recovery-acceptance.json`、测试日志及 `.symphony-evidence.json` 记录实际运行；最终发布 SHA 和源码文件清单另外绑定，不把开发基线 SHA 冒充发布 SHA。正式精确提交 Harness-Gate/CI 由独立宿主执行。

生产仍需提供：管理员操作 UID/完整 writer units 与 cgroup 约束；四类真实目录和挂载身份/配额；DB 最小权限和 TLS；独立密钥 custody；真正离机 mTLS 目标及服务端 ACL/配额/孤立对象生命周期；独立恢复数据库和禁外部出口策略；已验 release SHA/二进制/checksum；实际停机、升级迁移、切换/回退窗口和现场恢复验收记录。缺任何必要输入保持关闭或拒绝，受控 fixture 不能替代真实部署证明。

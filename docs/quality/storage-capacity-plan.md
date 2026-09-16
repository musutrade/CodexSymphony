# Symphony 磁盘容量调整方案（已选择，待管理员挂载）

## 2026-09-16 实测

| 挂载点 | 介质 | 文件系统容量 | 当时可用 |
|---|---|---:|---:|
| `/` | SSD | 216 GiB | 约 22 GiB，随构建变化 |
| `/mnt/dev-ssd` | SSD | 423 GiB | 163 GiB |
| `/data` | HDD | 916 GiB | 831 GiB |

Symphony 数据目前全部写入系统盘。故障时其目录约 49 GiB，其中 Gate
历史运行约 27 GiB、GH-17/18 工作区约 12 GiB、历史采集验收约 8.2 GiB。
本次已回收约 7 GiB 工作区普通编译与增量缓存；覆盖率二进制、原始
profile、报告、源码、签名、批准记录均保留。

增长来自独立编译、LLVM 覆盖率构建和为审计保留的原始证据，不是 PR
文本。部分单次 Gate 运行即使删除 target，仍保留约 1.2 GiB 的后端
采集证据。因此只清理缓存或不断增加磁盘都不能替代保留策略。

## 已选择布局

- 活跃运行数据：`/mnt/dev-ssd/codexsymphony-state`。
- 原有入口：`/home/gem/.local/share/codexsymphony`，使用持久 bind mount
  保留绝对路径，不使用会触发路径安全校验的软链接。
- 完成运行的压缩归档：`/data/codexsymphony-archive`。归档以 run ID /
  commit SHA 索引，包含文件清单与 SHA256，支持恢复原布局后审计。
- 报告、签名、审批、来源身份和归档索引长期保留在线。原始大文件按保留
  窗口转入冷存储，不因 PR 数量增加持续占用 SSD。
- 初始保留最近 10 次且不超过 30 天的完整热证据；其他完成运行转入
  校验过的归档。失败/中断运行也需归档，不能无限堆积。涉及活动任务、
  已打开文件或未完成证据写入时拒绝归档。
- 自动回收编译缓存，每小时尝试归档；启动要求至少 20 GiB 可用空间，
  运行中低于 12 GiB 暂停托管服务。当前未实现每类数据独立字节配额。

归档程序、恢复校验和定时任务已准备，尚未执行真实归档或修改挂载。
机械硬盘用于冷证据，活跃 Rust 构建留在 SSD。

## 可验证迁移步骤

1. 等当前 CI 完成，停止调度器、Gate、诊断/清理定时器及 fixture broker；
   核对无遗留 worker。记录各服务原先的运行状态。
2. 在目标 SSD 创建仅当前用户可访问的目录，检查目标挂载确实存在、
   容量足够。使用 rsync 保留权限、硬链接、时间戳和稀疏文件。
3. 再次停写后做最终同步和 checksum dry-run；必须无差异。
4. 保留原目录作为回退副本，以管理员权限建立 bind mount。验证路径、
   文件清单、权限、哈希和真实 sandbox/fixture/preparation 探针。
5. 配置开机持久挂载及服务的挂载依赖。磁盘缺失时必须拒绝启动，不能
   静默写回被遮盖的系统盘目录。
6. 恢复原先启用的服务，跑一次精确 SHA 的完整 CI。确认成功后再按明确
   的清理步骤释放系统盘旧副本；不要仅挂载遮盖后误以为空间已释放。
7. 回退：停止相关服务，卸载新挂载，恢复原目录和原挂载配置，再启服务。

当前障碍：普通账户可写入两块数据盘，但持久 bind mount 和挂载配置
需要管理员权限；`sudo -n` 返回需要交互认证。不能把尚未完成的迁移
说成已扩容。原始证据归档已实现清单、校验和恢复；定时任务等挂载后启用。

## 已准备的执行入口

用户已选择 SSD 运行区 + HDD 历史归档。

- `python3 tools/migrate_symphony_storage.py prepare`：停止相关服务，最终
  rsync 同步并做逐文件 checksum 比较，写入仓库外迁移记录。
- `sudo /usr/bin/python3 /home/gem/CodexSymphony/tools/migrate_symphony_storage.py activate`：
  校验准备状态、服务已停和副本未变化，创建持久 bind mount；原件保留，
  服务继续停止，等待真实沙箱及 CI 验证。
- `python3 tools/migrate_symphony_storage.py finalize`：只有迁移后出现新的
  完整 CI PASS 才允许释放系统盘回退副本。
- `python3 tools/archive_gate_evidence.py`：只列出归档候选；加 `--apply`
  才执行。当前归档范围是最大宗的 `probes/backend` 原始数据，诊断日志
  仍保持原位置。压缩包逐文件哈希校验通过、源文件未变化且没有活动
  Gate 后才回收热副本。`archive.json` 和冷盘清单保存恢复信息。
- `python3 tools/archive_gate_evidence.py --restore <原 run 目录> --destination <新目录>`：
  校验压缩包及逐文件摘要后恢复原始文件及权限，不覆盖现有目录。
- `tools/install_evidence_archive.py` 安装每小时归档任务；SSD 绑定挂载
  和 `/data` 都存在才可运行。默认保留最近 10 次、30 天内热证据；活动
  Gate、刚结束不足 1 小时的运行一律不归档。

注意：其他类别的历史验收数据仍需单独纳入生命周期；归档也会占用
HDD。容量监控和低水位停止继续保留，不能把“831 GiB 空余”说成无限
容量。未来应根据实际每周增长量调整冷热保留期限或备份容量。

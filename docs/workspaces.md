# 工作区、Git Broker 与保全（GH-15）

本项实现第 8、10.1–10.3、14、17 章的本地工作资料边界，对应 A03 工作、A04 恢复、A06 路径/身份部分。不会因本地提交存在而宣称远端分支、PR、验证通过或业务完成。

## 平台调用路径

`workspace_store::execute` 是持久化与授权入口，复用执行控制、暂停、撤销和评审的 PostgreSQL 锁。输入绑定 Run/request/incarnation、Requirement/revision、workspace identity、当前 phase、全局归属和执行状态。Prepare/Restore 要求 Created，Commit 要求 Running/execution；Preserve 必须有已持久化的整个旧执行组静止事实，暂停或撤销后仍允许保全。旧 Run 不能覆盖新 Run 或当前阶段。Git 与文件操作在阻塞线程执行，协调器 tick 和 HTTP 线程不等待同步文件 I/O。

本地适配器 `GitBroker` 接受平台提供的本地 Git bundle，创建全新的 canonical 裸仓库。运行根目录布局：

```text
<execution-root>/workspaces/
  canonical.git/                 平台 Git 对象、refs、worktree 元数据
  runs/<unique-run-id>/          唯一 Run cwd，包含受保护 .git 指针
  archives/<source-run-id>/
    pending                     文件存在不代表数据库已接受完整引用
    history.bundle              HEAD 全部可达提交与 index tree 对象
    index                       原 index；不复制其他 Git 配置或 hooks
    files/                      工作文件，含未跟踪/忽略的源码、测试、进度
    manifest.json               SHA-256、Git 执行位、排除项与来源身份
```

先调用已有内部 Run reservation（launch cwd 必须等于 Broker 计算的路径），再 Prepare 或 Restore。后续 Runtime 必须把进程与 thread cwd 都设置为该路径，canonical/archive 由平台保管，worktree 中普通本地 Git 可写，不以只读 `.git` 作为产品要求。此项没有复制外部开发控制器的 Git Data API 发布协议，也没有宣称同 UID 的强读隔离已完成。

固定 `/usr/bin/git`、参数数组、清空继承环境、禁系统/全局配置，并逐次核对 canonical 的最小受管配置。禁 hooks、credential helper、签名、fsmonitor、自动维护及非本地协议；只从已验证的本地 bundle 导入。不运行 Agent 的 Git shell，不提供 push。Commit 由 Broker 暂存源码变更（沿用 Git ignore，并明确排除三个根缓存目录）后提交；消息通过 stdin 输入，结果由 Run/tool request 幂等保存。Git 对象/refs 开启 fsync。

## 文件与数据库之间的失败边界

1. 独立事务保存 pending 意图；重复参数必须一致。
2. 再次锁定并核对授权，执行 Git/文件操作。创建必须排他，不复用旧路径。
3. 写入、fsync 并回读校验所有文件；校验 bundle、HEAD、index、工作内容及清单。
4. 同一数据库事务保存身份或清单引用及 complete 结果，成功后才回复。

进程退出或 DB 不可达时 pending 保留。可记录失败时改为 partial 并阻止续写。无论失败点在哪里，原 worktree 和部分目录都保留；没有 reset、删除、自动清理、自动重跑或自动把残留 manifest 提升为成功的路径。`pending` 文件故意保留：它说明必须查询数据库状态，不能只凭文件名判定封存成功。

恢复协调器在旧组静止后为已登记工作区保全。完整快照缺失或任何操作仍 pending/partial 时，启动恢复屏障不能打开。残留资料需要显式对账；本项不提供会盲目重试的修复按钮。

## 保全与恢复内容

- 历史 bundle 独立包含 HEAD 的可达中间提交及保存 index 所需的对象。执行恢复以保存 HEAD 为新基线，恢复原 index 与每个工作文件，核对 SHA-256 与 Git 执行位；不读取最新 main 作为隐式基线。
- 仅排除工作区根部 `target`、`node_modules`、`.angular`，清单记录实际排除项。若其中存在当前 tracked 文件则拒绝保全，不静默丢实现。其他 ignored 文件也保存。
- 不跟随符号链接；遇到链接、子模块、非普通文件、非 UTF-8 路径或不能 write-tree 的冲突 index 时拒绝并保留原件。已知凭证文件名、嵌套 `.git` 和 Git 配置文件拒绝复制；检查所保全历史中的对应路径。它不是任意文件内容的秘密扫描器。
- 工作恢复选择最近来源的当前 revision/phase。待验证时只恢复仍有效的 candidate HEAD 的干净树，完整脏工作仍留在原档；候选失效或来源已被后续 Run 替代时拒绝。待交接必须有独立验证身份，本项保持未就绪，不从 manifest 推断 PASS。

## 实际验收与未接通边界

`apps/server/tests/workspaces.rs` 使用真实 Git 和独立 PostgreSQL schema：staged/unstaged、未跟踪源码/测试/进度、二进制、执行位、删除、中间提交、替换 canonical 后恢复；恶意 hooks/helper、链接、错误归属；重复与 stale 操作；真实 SQL 写入失败；写文件大小限制导致的真实部分写入；文件写完后的子进程退出与重启对账屏障。

Runtime 动态工具、完成声明受理、受信验证、预算/磁盘预检、远端获取与 PR 交付仍属于后续 Issue。Ready 不因此开始真实编码；0a 的整体 A03/A04/A06 和 DoD 不因本项自动完成。此产品 Broker 与开发本仓库的宿主发布工具是独立实现。

### GH-86 真实 Runtime 验收恢复

本地提交暂存使用 `ls-files --cached --others --exclude-standard -z` 选择源文件，
过滤固定缓存根后，以 NUL 分隔的 literal pathspec 交给 `git add --all`。
这保留跟踪文件删除、特殊文件名和忽略规则，避免负目录 pathspec 在已忽略的
`target` 存在时返回失败，即使源文件已经进入暂存区。

最新 Run 已静止、Interrupted 且工作区操作仍 pending/partial 时，任务组进度返回
`workspace_reconciliation_required`。这只是阻塞原因，不释放 owner、不伪造 Done、
不自动重放部分操作。外部验收调用方应同时检查 Run 和交付记录，不能只等待
Requirement 的 Running 字段变化。

GH-86 合并收尾不申请新模型轮次，因此使用已有预留额度的 `prepaid_fits` 检查。
最后一个已授权轮次恰好占满预留额度时仍可完成合并与验收；已耗尽标记、实际/预留
超出子项或父组上限仍拒绝。此恢复不增加、清空或重置任何预算。

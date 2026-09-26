# GH-122 双路径验收证据

范围及 AC 对照见 [双路径验收](../../dual-path-acceptance.md)。开发基线为
`9edde80f5938fd48b3bdac7f3148e3bb19125185`。本目录只保存可提交的精简证据；
数据库、认证配置、完整 API 响应和原生覆盖率产物保留在独立宿主，不能复制凭据进仓库。

## 真实任务与受控测试

[local_git 原始验收](local-acceptance.json)记录线上模型生成代码、实际 Git 更新、
独立 Node 验收与 Done。原调用使用各自记录的二进制；后续新版读取 Done 是持久性验证，
不追认为在新版上重新完成线上编码。原失败、两次模型调用及未知用量预留均保留。

GitHub 真实任务的候选为 `d4fca58879b78f997ebde0b98317c2287642fd1e`。
宿主断言成功但无输出，核心按协议记录 unknown；明确授权的新计划只在原断言成功后
输出结果。后继验证沿用原候选、Run、Requirement 和预算，旧验证保持失败。
随后发现存储汇总的单验证假设，诊断及零发布尝试见
[存储恢复前事实](storage-revalidation-recovery.json)。暂停又使成功验证失效，恢复需显式生成新证明；确认零发送的操作才可更新验证引用，并追加审计记录。旧失效标记、验证内容和累计预算保持。[GitHub 最终结果](github-acceptance.json)确认原任务 Done：PR #12 经原保护规则合并，实际合并版本 `3e7befaf0b5bdc835854cc1d5a14c62865aec4c4` 独立验收通过。最终二进制重启后仍为一次模型调用、一次交付、一次合并和一条验证引用更新审计；两次发布尝试分别为 push、create。两条隔离验收服务均已停止，数据库及原始证据保留。

原始证据路径及摘要见 [保留产物索引](retained-artifact-hashes.json)。
受控 PostgreSQL、真实子进程和协议 fixture 的测试只证明相应边界；
它们不冒充线上模型、GitHub 外部写或手机验收。

## 测量与授权

- [最终 Rust 源码测量](rust-measurement-summary.json)：2,223 个生产函数，373 项测试／41 组；行和区域覆盖率均至少 80%，CRAP 均不超过 10。完整测量见压缩 JSON，输入摘要见 [源码输入](source-inputs.json)。[Rust 检查](rust-checks.json)在测量 PASS 后执行并通过。
- [前端测量](frontend-measurement-summary.json)：23 个文件、189 个函数，
  保留精确源码绑定；当前改动后须核对来源摘要。
- [三个升级 Python 入口](python-upgrade-measurements.json)：使用用户批准的
  coverage.py 7.16.1 / Radon 6.0.1 任务范围方法。原测量中的“approval required”
  是采集器固定文本，实际批准见 [授权原件](authorization.json)。该批准不构成永久策略。
- [回收优化源码核对](retention-source-match.json)：复用的是完全相同源码自身的测量，
  不借用其他来源证据；组合提交仍须完整 Gate。
- [预算授权](budget-authorization.json)是应用前的授权快照，保留当时的 `applied: false`；
  是否实际应用以原账户版本和真实结果中的 limits、usage、reserved 为准。

[采集时间索引](capture-timing-index.json)直接来自保留的宿主阶段事件。这里的 capture PASS 只表示采集进程成功，不等于测量阈值通过；定向采集和失败重做另有原因记录。新增测试格式遗漏及准入静态复核导致的重做也保留在索引中。

每次失败的采集和检查均保留，不覆盖为成功。单项测量、编译成功或真实小任务 Done
都不能代替最终精确提交的完整 Gate；最终 Gate 回执在宿主保存并由 PR 引用，避免
把包含自身提交号的回执写回该提交。

## 人工介入及边界

人工介入包括环境探针竞争修复、原账户追加预算、成功断言输出修正、同候选恢复授权、暂停后零发送操作的显式重验
和多验证存储兼容修复。任务不属于无人介入验收。cached tokens 是 input 的子集；
`usage.complete: false` 时观测数只是下界，不能释放未知调用预留或推断最终费用。

不宣称 #102/#83/#90 父组、#89 手机全流程、#126 完整诊断或生产现场全部完成。
已接受新恢复动作的数据库不能直接降级给不理解该动作的旧二进制；
任何回退先停止并保全原任务和副作用事实，不通过新建任务清零。

# GH-90 组合验收证据

任务原基线见 [task.json](task.json)。早期验收使用独立 clone；当前开发已归并到
固定 `/home/gem/CodexSymphony`，不再按尝试创建工作区。当前宿主整改与原账户恢复
见 [实施边界](../../proposals/current-development-storage.md)。下文早期结果均保留其原来源身份。

本目录是原件索引，不是受信发布回执。覆盖率/CRAP、普通检查、组合验收和最终完整 Gate
分别取证，缺少任何一层不能从其他层推断通过。AC 对照、使用/迁移/回退和真实试用模板
见 [V1 发布索引](../../v1-delivery.md)。#90 的真实组合链及生产现场责任保持开放。

本轮运行根目录：`/home/gem/.local/share/codexsymphony/gh90-development`。
只保留脱敏摘要和原件哈希，数据库、会话、平台密钥、模型认证不进入源码工作区。
所有 fixtures 标明合成输入；真实服务/进程/数据库/HTTPS/Git 并不自动意味着线上模型、
外部 GitHub 写入或实体手机通知已经验收。

最终发布身份由安装宿主保存于
`/home/gem/.local/share/codexsymphony/publication/GH-90/receipt.json`，
其精确树、环境、审批和完整报告必须全数匹配。本目录不将运行后的回执写回被其绑定的树。
开发过程和失败恢复遵循 [AGENTS.md](../../../AGENTS.md)；旧失败原件不被成功重跑覆盖。

## 本轮结果

[后端测量](backend-measurement.json)是新采集及独立 LLVM 重导出的 2,309 个生产函数，
[前端测量](frontend-measurement.json)覆盖 26 个文件、230 个函数；声明文件按批准的分组处理。
覆盖率和 CRAP 均 PASS、零违规，没有复用前置任务的测试结果、排除源或调整阈值。
已完成的手工 Rust capture 注册于宿主缓存保留工具，原始计数、对象和测量仍在原路径。

[执行索引](acceptance.json)保留格式、Clippy、秘密扫描、lint、构建和当前二进制来源。
真实 HTTPS 合约的 108 个响应变体通过；桌面/移动浏览器 34 条通过，页面与业务操作
使用实际 Rust API。认证撤销/限流的多次真服务重启、三实例手机恢复，以及通知/执行隔离
14 条回归通过。脚本 Runtime 的三次 turn 保持为 fixture 事实，不计成线上模型调用。
新编译的同源产品二进制另通过 GitHub App 只读能力预检，没有本轮仓库写入。

两个隔离迁移库分别从 M2 24 条和重构后 39 条迁移升到当前 40 条；首 24 条 SQL
逐字节核对 M2 历史提交，旧 SQLx checksum 保持。四次实际产品启动比较原表列的
行哈希，授权、用量/未知预留、修复记录、保存工作、取消事实与交付记录均保持。
39 条样本另保留本地版本、验证后继/代次、未知 Hook 与通知尝试/作用域版本。
原部分保全操作使恢复闸门保持关闭，队列仍暂停；没有伪造恢复完成或创建新 Run。
这些是合成持久事实的升级检查，不是带真实在途外部副作用的生产切换或旧版本降级证明。

备份三个不同原测试场景均有成功结果：加密归档经真实 mTLS 测试对象服务传输、校验、
隔离 PostgreSQL/Git/通知账本恢复及两次只读恢复启动；损坏、密钥错误、材料缺失、
容量/锁/权限失败和 CLI 拒绝也通过。生产离机目标没有配置或执行。

## 失败与恢复边界

[恢复索引](recovery.json)列出六个有计时的失败阶段、一个命令前数据库前提失败和
六项 operator 恢复决定。原 `timings.jsonl`、stdout/stderr 与尝试报告全部保留。
失败不改写为成功；部分通过的迁移/备份场景按各自原件取证，后续只重做未通过的场景。

Nginx 早期入口检查因缺少可执行文件失败。2026-09-28 已核验历史保留包与原
选定版本摘要，使用相同程序补跑隔离验收通过，结果位于受限存储
`evidence/manual/gh90-nginx-da30214876fa/`。新尝试首轮缺少 Python 模块路径的失败
另行保留；补充结果不覆盖早期失败，也不代表公网或生产入口已验收。

备份旧测试 helper 在 HTTP listen 后立即停服务，与现行 fixture 的恢复闸门要求不一致。
operator 仅为测试 context 增加实际恢复/存储状态只读等待；没有直接更新 readiness 标志。
`pg_dump`/`pg_restore` 来自已经批准的一次性 PostgreSQL 镜像，逐文件 SHA256 一致，
以只读测试 mount 使用。随后原 `target` 符号链接被正式备份配置正确拒绝；测试配置改为
相同二进制的实际规范路径，原私有路径/身份检查继续生效，未修改生产备份代码。
原失败/限额和 CLI 两例已通过，最后只重做失败的恢复例。

迁移 operator 的第一个读取器误用了返回 None 的写入 helper；改为读取真实 psql
stdout 后 M2 样本通过。39 条样本的通知 fixture 被原作用域触发器拒绝；显式登记
关闭的通知作用域及原 scope_version 后再验证，既有产品权限检查没有放宽。

## B01 原父组真实链完成（2026-09-28）

[紧凑原件索引](b01-real-group.json)绑定原 draft、一次整组授权、实际模型身份、调用账本、三次交付和最终纯验证。原需求没有重建；两次预算停止、两次原账户扩额、旧失败及未知预留仍保留。实际 Runtime 身份五次均匹配 `gpt-6-astra/low`，不是脚本对端。

| 子项 | 实际交付 | 合并与验收 SHA | 结果 |
| --- | --- | --- | --- |
| C1 normalize_label | [PR #13](https://github.com/musutrade/disposable-2/pull/13) | `a0d31908e02715869243321930d03c50f45a9d6c` | Done |
| C2 parse_labels | [PR #14](https://github.com/musutrade/disposable-2/pull/14) | `b05a8d07bcc1d39fe3d155fb813a464f2cf07381` | Done |
| C3 summarize_labels | [PR #15](https://github.com/musutrade/disposable-2/pull/15) | `8870ba39b52952e5b2f2f14745292646f776b2c1` | Done |
| C4 validation_only | 无提交、无 PR、无模型调用 | 最终 C3 合并版本及前三项交付材料 | Done |

三个代码项严格串行，各自经过 CI、test-merge 验证、受保护 squash 合并及实际合并版本验收。C4 在最终版本执行 9 项 locked/offline Cargo 测试，包括组合场景；前后源码 tree 相同。父验收绑定三个原交付材料和 P1–P4，API 返回 `parent_state=Done`、`business_complete=true`。本次录入/授权通过本地控制面 API；桌面/移动浏览器检查按前述 fixture 边界单列，不冒充实体手机操作。

账本累计 407,981 tokens（input 已含 cached）、5 turns、230 已知模型秒；另保留 670 模型秒未知预留。各子项均在最后明确批准的额度内，C4 零用量。不得把 Runtime 的 `complete=false` 记录改写为完整账单或清零未知值。原需求创建至父验收为 53,580.13 秒，包含等待授权、宿主整改、质量验证与恢复；最后一次恢复到父验收为 1,302.09 秒。这些墙钟区间不与已知模型秒相加，不能据此推导零介入或通用性能。

账本有 7 条操作 API 介入记录（1 次准备重查、2 次暂停、2 次恢复、2 次保全恢复），另外记录 2 次原账户扩额及 1 次 cc/rg 环境修复；不将该分类冒充全部人工工作的总次数。编码/验收使用 `run-ae5cb2017a43` 完整 Gate 通过的二进制，摘要见索引；最终发布仍须绑定所有后续文档的精确树。

## 仍需完成

2026-10-01 增量见 [R2 原交付重验索引](r2-original-delivery-revalidation.json)：
用户真实重启后，fc8d930f 使用原请求取得原生静止证明，原 unknown 不变；
R2 同候选新验证和原 recheck 实际通过，Req6 Done，新增模型调用和 Git 更新均为零。
首次受保护停机实际通过。队列模型修复的完整诊断 Gate `run-ced10750ef66` PASS，
随后仅原 R3 更新到 v2 / revision 2，R1/R2 历史完成和原账本保持。
[R3 首次组合失败索引](r3-original-integration-classification-failure.json)记录规范 Node
TypeError 的真实失败及原生分类缺口；未生成关联修复，累计 calls 仍为 9。
当前服务受保护停止、全局暂停；新分类修复仍需精确源码测量和完整 Gate，R3/GH-90 未完成。

用户后来已授权 GH-90 专属 disposable 范围与有限模型预算，原父组已建立，C1
已开始真实调用与交付；不得再按早期“零调用、无授权”摘要处理。实际用量、未知预留及
已批准扩额见当前实施边界和原账本。B01 三代码子项/纯验证父组已完成；真实合并后与集成失败
修复、混合父组、线上诊断读取及其余 GitHub 故障组合尚待完成，不能据受控回归勾过。
M2 公网、实体手机/Bark、离机恢复及生产切换责任继续见原 [上线清单](../../m2-delivery.md)。

[环境](environment.json)、[源码输入](source-inputs.json)和 [计时索引](timing-index.json)
提供版本与原件来源。最终 replay helper 另保留在运行目录，不追认其为早期失败 operator
版本的精确身份；生产与仓库测试源码由独立 collector 输入摘要绑定。
计时用原阶段区间合并计算重叠一次；准备/等待空档不回填成执行，最终 Gate 单独由宿主计时。
本轮已有上述一组真实交付样本；它不代表后续五类试用都已执行，也不用于计算零介入率或外推交付效率。

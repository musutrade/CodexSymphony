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

## 早期受控结果（保留原版本身份）

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

## 原 R3 关联修复及混合父组完成（2026-10-01）

[R2 原交付重验](r2-original-delivery-revalidation.json)、[R3 首次组合失败](r3-original-integration-classification-failure.json)、[本地验收纠正](r3-local-acceptance-correction.json)和[未启动证明恢复](r3-unstarted-validation-context-recovery.json)保留各阶段原件。规范 TypeError 分类后，首次模型调用因上游认证失败留下未知预留；用户明确向原账户追加 120,000 tokens，正常 storage-recheck 关联的后继完成两文件修复。没有新建账户、清空预留或创建 ordinal 2。

同一 Node 候选 `0f446bb795799cd753aa84651f1eac6fd0f55e68` 通过纠正后的本地验收，原交付 passed/released/quiescent。随后组合检查 `integration-47062b9e-befc-4089-acee-afe9735beb52` 实际通过，绑定 Rust `9d77f4ff9d508e7c28a7d0bb76a2d3fec964adf6` 与上述 Node 候选。Req7 revision 2 为 Done；原父组 API 返回 `business_complete=true`、`parent_state=Done`、3/3。旧 integration 失败和五个证据文件不变，原关联失败为 complete，post-local 阻挡为 cancelled。Req1–6 的原完成事实不变。

全库累计模型调用 11 次（B01 五次、混合组六次）。最终证明恢复、本地纠正及组合重验新增模型调用和 Git 重发均为零。混合组已知 352,561 tokens / 6 turns / 205 模型秒，另保留 367,439 tokens / 875 秒暴露；父上限 720,000 / 6 / 1,080，R3 上限 240,000 / 2 / 360。未知记录不因 Done 结算，当前没有新增付费调用授权。完成后全局暂停，owner 为空。

本次精确源码测量 `run-546435187777` 为 2,442 函数、零违规、max CRAP 10；完整诊断 Gate `run-b79591685971` PASS。部署 commit 为 `c50c0a4e`、tree `4316f9c1`、binary `59d0c4a4`。用户授权 Codex 接手 Claude 网关故障后的工作；Codex执行冻结审计脚本并独立现场核对 27/27，不将其冒充新的 Claude 审计。诊断回执不替代包含本页的最终发布 Gate。

## factory 入口及剩余现场边界

[公网入口索引](factory-ingress.json)记录 `https://factory.aglmud.org` 实际 HTTPS、正常账号登录、Secure Cookie、退出后旧会话拒绝、匿名及伪造 Access 头拒绝、CSRF/Origin 拒绝和静态产物核对。原产品只监听 loopback 3081，现有 Tunnel 沿用 8790；未修改远端 Tunnel。切换前备份、失败首轮回退及业务/账本逐行比较均保留。

此入口的本机段为 loopback HTTP，未安装 M2 要求的 origin TLS8443；用户级网关与控制面共享 UID。公网功能通过不等于完整 M2 部署通过。实体手机、Bark 专用 UID/网络与渠道观测、实际离机目标/独立 AES custody 仍待完成；未把本机备份或 mTLS fixture 当成离机生产备份。

B05/B06/X01 的这个真实混合样本已完成；B02/B03/B04/B05/B07/B08 与 X02–X05 的剩余组合仍按 [V1 索引](../../v1-delivery.md)开放，GH-90 不据此关闭。

## X04 集成诊断连接（2026-10-02 开发）

补充父组集成 invocation 的诊断登记及原项关联修复读取关系，见 [诊断说明](../../diagnostics.md)。新增组合用例 `integration_reports_survive_cleanup_and_only_the_linked_repair_can_read_them` 经真实监督器生成四份产物，清理原检查目录后，通过产品 Runtime 工具逐段重建约 143 KB 日志、84 KB Markdown 和 110 KB 单行 JSON，核对首尾、长度及摘要；未关联读取和解除修复权限后的读取均拒绝，原集成失败结果保持。授权关系为隔离数据库中的明确 fixture；没有付费模型、现场部署或真实业务写入。

首轮捕获 `run-01790eda8595` 的 44 组、460 项测试通过，独立重导出 2,443 个生产函数覆盖率/CRAP PASS、零违规，已按安装宿主登记。共享测试 helper 引用随后整理，并增加 source/invocation 交叉替换拒绝断言；该首轮证据不替代修正版本的测量及最终精确树完整 Gate。原件位于固定宿主证据目录，阶段时间沿用各自 `timings.jsonl`。

此连接补齐受控组合边界。线上修复 Agent 读取长报告/附件的独立真实恢复验收仍开放：当前原账户无新增付费调用授权，11 次历史调用、未知暴露和全局暂停保持。没有据此关闭 X04 或 GH-90。

[环境](environment.json)、[源码输入](source-inputs.json)和 [计时索引](timing-index.json)提供各自原版本与原件来源。计时仍使用原阶段边界，不把等待回填成执行，不据这一组样本外推零介入率或交付效率。最终精确树发布身份由仓库外宿主回执保存。

后续同源采集 `run-e48c970cb36d` 的 44 组 / 460 项测试通过，原生对象打包因 ENOSPC 失败，失败日志未改写。使用批准的保留维护整理已完成的前次采集（前次 raw/tmp 已不再在线），保留失败部分对象，将未变动的原编译对象逐项摘要核对后补齐；批准的 `recover_capture.py` 完成打包恢复、独立重导出和保留登记。其 2443 个函数 coverage/CRAP PASS、零违规，仅绑定恢复时的冻结源码，不作为后续多仓库读取修正或最终 Gate 的证据。宿主记录 `recover-partial-native-objects`、`recover-native-packaging`、`backend-source-measurement` 和 `capture-source-handoff` 时段。没有重复测试或修改受信宿主。

诊断与恢复组合另补充完整冻结仓库集合的读取范围检查及受控双仓库回归：产品和 Runtime 都拒绝次要仓库撤权的报告，也拒绝重新授权后属于旧授权版本的报告；修复 reservation 结束后再次拒绝。报告目录删除后仍按 8192 字节分段核对全部内容与摘要，原业务结果保持不变。多仓库修正后的独立测量见下述记录；现场模型验收仍待原账户授权。原账户本轮只读核验保持 11 次模型调用、paused=true、owner=null。

多仓库修正的采集 `run-469a6017d054` 完整通过 44 组 / 460 项测试；独立原件核验与测量覆盖 2443 个生产函数，coverage/CRAP PASS、零违规。新增登记函数行与 region 均 100%，CRAP=2；两个读取函数行 100%、CRAP=7/8；集成完成函数行 90%、CRAP=201/40。测量 SHA256、精确源码摘要、登记身份、四份产物身份/摘要及原失败保全事实见 [受控组合记录](x04-integration-diagnostics.json)。这次和前次完成采集的 raw/tmp 已按批准策略回收，不能声称仍在线；保留审阅归档、测量、源码交接和应用证据，审阅归档不包含完整原生对象。

测量后格式与所有目标/功能的 Clippy（拒绝 warnings）通过；配置中的秘密扫描零发现。独立扫描包装先后缺少固定工具 PATH 和可写报告目录，保留两次环境失败日志；采用安装宿主已有的 tool_path 与报告目录挂载后通过，没有改动源码、扫描策略或受信程序。最终精确树完整 Gate 由安装的 publication host 另行绑定并保留回执，本受控记录不替代该回执。

## X02 旧验证代次写入边界（受控验收）

[源码绑定证据](x02-validation-generations.json)记录隔离 PostgreSQL、真实 Git 与验证进程下的暂停/恢复、取消及代次替代组合。旧代次的开始、迟到 step、重复完成、hook 评价覆盖和代码修复预留均被拒绝，原状态与输出保持；后继在独立目录实际运行验证后完成自己的交付。

本轮 44 组、461 个测试通过；2,442 个生产函数的源码覆盖率与 CRAP 测量零违规，随后格式、Clippy、秘密扫描和架构检查通过。首轮新增夹具误建多个初始验证而失败，已按产品 `retry_of` 关系纠正，旧源码和失败日志保留。本轮没有线上切换或新增业务模型调用，不把受控测试计作新的现场验收；X02 其他故障组合和 GH-90 剩余项仍开放。最终发布身份以精确树宿主回执为准。

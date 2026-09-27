# GH-89 验收与来源

实现基线是 #126 / PR #137 合并后的 main `3060e2851cde64f23fee8399cb6cae85f3570877`。
开发使用独立 clone 和 `feat/gh89-mobile-lifecycle`，未覆盖原工作区未提交内容。
依赖 #106、#128、#126 的产品接口与状态机直接复用；没有重新授权业务任务、部署宿主或启动生产队列。

## AC 与执行证据

| AC | 实现／回归主体 | 本轮保留证据 |
| --- | --- | --- |
| AC01 | `operator_execution` 只读输入／父子账本；详情 Run、验证代次、GitHub／本地交付、关联修复与验收独立展示 | Rust `operators`、`local_git`、`linked_repair`、`integration_validation`；桌面／移动 operations 浏览器结果 |
| AC02 | 冻结环境／Hook 身份、历史作用域版本和当前授权；环境原观察／差异，真实阶段耗时及 unknown／not_applicable | Rust `operators`、`extension_lifecycle`；详情与真实 HTTP 合约 |
| AC03 | `RecoveryPanel` 对当前对象／revision／验证的决定；同候选、交付重验和范围内适配；稳定请求 ID、保存决定与后继记录 | Angular recovery-panel；Rust `extension_lifecycle`；浏览器恢复幂等、409、预算未清零 |
| AC04 | 复用 #126 `DiagnosticPanel` 与受保护清单、完整下载／完整性／缺失原因，保留六问 | Rust `diagnostics`；桌面／移动完整下载回归 |
| AC05 | 复用所有阶段暂停／取消／恢复 API、静止保全与外部对账；确认文案保留已合并事实 | Rust `delivery`、`automatic_merge`、`integration_validation`、`bounded_recovery`；浏览器迟到合并事实／取消占用；独立真实服务重启场景 |
| AC06 | 复用 M1 原差量变化评审、冻结输入及领取保护；保持本地草稿与禁止认证恢复自动写 | Angular group-review／operations；Rust `group_edits`／`group_queue`；auth、队列差量浏览器回归 |
| AC07 | `LifecyclePanel` 持久序号分页、有界补投、原事件和尝试次数；插件状态与渠道事实分开 | Angular lifecycle-panel；Rust `extension_lifecycle`；真实 API 浏览器补投／撤权／没有额外模型调用 |
| AC08 | 子项显式 provider／model／effort 评审；冻结来源／实际身份／匹配及每次用量显示 | Angular group-review-model；Rust 模型身份回归；浏览器不支持配置被拒且输入保留、fixture 身份不匹配／未知预留 |
| AC09 | 真服务 HTTPS 的 desktop 与 Pixel 7，认证过期、断网、诊断、队列、恢复与通知；独立隔夜／重启 | 本目录结果摘要、保留浏览器截图／附件、HTTP collector 合约证据与 mobile recovery 结果 |

执行结果、测量报告 SHA256、命令、开始／结束时间和源码输入另存本目录机器可读摘要。
最终精确树完整 Gate 与发布准入以宿主持有的
`/home/gem/.local/share/codexsymphony/publication/GH-89/receipt.json` 及其报告引用为权威。
该回执在全部源码、文档和本目录证据冻结后取得，不再把运行后的回执写回被它绑定的树。

## 本轮已完成结果

精确 source-bound 测量覆盖 Rust 2309 个函数、TypeScript 230 个函数：覆盖率与 CRAP
均 PASS，全部规则无违规；报告引用、SHA256 及改变函数的精确值见测量摘要。
真实 HTTPS 合约的 108 个响应变体、脱敏后的相同校验与完整 Angular 消费端清单 PASS。
格式、Clippy、秘密扫描、frontend lint 与生产构建通过，策略和失败阈值保持原审批身份。
生产构建出现既有 500 kB warning 预算提示（初始 bundle 约 552 kB）；未改预算，构建成功。

桌面／手机完整浏览器套件 34 条通过。之后清理草稿旧提示并明确缺失诊断引用，重测量后
再跑受影响的草稿与 operations 页面 12 条，全部通过；未重复其他已通过且源码未改的流程。
真实服务重启场景也通过：3 个服务实例、3 个 Run，旧问题／工作／原账户和授权保留，
暂停重启不继续，明确恢复后接续，取消后全部进程静止、收尾完成再释放占用。
所有结果是本项源和输入的受控验收，不能替代最终宿主完整 Gate 或下述现场边界。

旧 Runtime 准备探针锁定旧 Core 身份；第一次重启验收在 Run 启动前被真实准备失败阻断。
受控 Runtime 子进程改用独立 home／PATH，指向已保留且精确 SHA256 匹配产品锁的实际二进制；
两条工具路径的版本／摘要由原环境探针记录，开发 Gate 的环境和审批保持其现行身份。
未修改准备探针、工具锁或受信宿主。失败保存于原 `artifacts/gh73/recovery`；成功尝试使用
独立 `artifacts/gh89/mobile-recovery`，具体路径与原始 JSON 摘要见 `acceptance.json`。

## 真实与受控边界

浏览器运行产品 Rust 服务、真实 PostgreSQL、HTTPS 和 Chromium，没有 mock 页面状态。
扩展故障、模型响应身份、通知失败和 GitHub 迟到观察是一次性数据库中明确标注的生产者 fixture。
恢复决定、版本冲突、仓库撤权、补投计数及取消对账由实际产品 API／协调器执行。
隔夜／暂停重启使用真实进程、Git 工作区、保全和同账户预算，Runtime 对端是脚本 fixture。
Rust `delivery` 的远端竞态使用受控 Broker；浏览器核对相同竞态的持久事实和控制行为。
这些结果不证明公网 GitHub/Bark、现场手机收到通知、真实模型质量或完整父子链现场矩阵；
原 #106／#122 现场模型证据保持其原身份，#90 继续承担完整链与现场矩阵。

## 测量兼容与失败保留

批准的 TypeScript 仪器会包裹字段初始化表达式，因此本轮 Angular 输入／输出使用原生
`@Input`／`@Output`，内部状态仍是 signals。没有变更 collector、排除生产源或降低策略。
批准的 HTTP generator 要求封闭 schema；恢复历史的可空引用及关联验收／版本字段按实际
Rust 生产者补齐，生成类型保留不变定义，未更改已批准健康接口基线。

首次 Rust 采集中的新测试缺少冻结 revision，违反外键；补齐真实冻结输入后重新采集。
第二次全量采集测试成功，但 `operator_view.detail` 的精确 CRAP 高于策略上限，测量仍记失败，
没有据此运行普通检查。将环境／执行投影附加拆为内聚命名函数，保留只读事务、顺序和错误传播，
随后用新目录重新采集。所有失败日志、源码快照、原始计数和测量保留于任务证据位置。
完成的 Rust 手工 capture 按宿主保留工具注册，目标编译缓存可回收，原始证据不删除。

HTTP 首轮原始响应通过校验，但既有短错误密码占位值会使脱敏后的 `hook_invalidated`
字段名改变；将错误登录用例改成既有策略认可的 `test-password` 占位值，保留全部认证
失败与限流变体。另将生命周期 cursor 放入标准 HttpClient `params`，适配批准的
消费端清单；没有更改脱敏器、collector 或质量策略。细节和各失败尝试见 `recovery.jsonl`。

## 迁移与回退

没有新增数据库迁移、依赖／工具锁变更、宿主策略变更或质量例外。新增只读字段是增量，
恢复／生命周期 API 保持原契约；纠正已有 operations 可空／关联字段的 OpenAPI 描述。
回退此产品提交可移除界面和投影；原有持久决定、授权、预算、事件、已交付版本和诊断仍保留。
不得通过回退删表、清零账户、撤销已合并事实或重新执行已取消工作。
操作说明见 [mobile-lifecycle.md](../../mobile-lifecycle.md)，执行顺序见 [AGENTS.md](../../../AGENTS.md)。

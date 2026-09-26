# GH-106 实施与证据索引

开发副本 `/home/gem/CodexSymphony-gh106`，分支 `codex/gh106-fixed-model`。
起点为 main `f29052fbcb4834cbfb1565a4f32a60c83ba00951`；#105、#122 已合入，
原工作区的未提交修改保持原样。范围、API、迁移及回退见[固定任务模型选择](../../fixed-model-selection.md)。

当前发布状态：完整 Gate 在宿主前置检查被拒绝，未推送或创建 PR；详见[具体阻断与恢复条件](publication-blocker.md)。

## 源码验证

[测量摘要](measurements.json)对应 `/home/gem/gh106-measurements/source-05` 和 `frontend-02`：
2,246 个 Rust 函数全部达到行/区域覆盖率至少 80%、CRAP 不超过 10；
前端 23 个文件、189 个函数按已批准清单和策略测量通过。
[输入摘要](measured-inputs.json)保留实际源码 SHA-256；[环境身份](environment.json)保留工具和批准身份。
生成类型沿用既有声明清单，不增加排除或降低覆盖率。

测量通过后，同源码的格式校验、Clippy（全部 targets/features，warnings 为错误）、
前端 lint 和配置的秘密扫描通过。隔离 HTTPS 实际响应及生成客户端的
[HTTP 契约测量](http-measurement.json)通过；[新增接口响应](http-model-response.json)
验证历史 Run 的未知身份仍为 null。前端源码只更新生成契约类型，不增加交互流程。

完整发布 Gate 必须另由安装宿主对最终精确树执行；本索引和上述局部测量不是发布凭证。
最终宿主回执位置为 `/home/gem/.local/share/codexsymphony/publication/GH-106/receipt.json`，
其中的树、环境、报告路径和报告摘要共同决定发布准入。回执不写回其绑定的候选树。

## AC 与证据边界

| 条件 | 已验证内容与证据 |
| --- | --- |
| E06-A | `group_review::fixed_models_are_reviewed_per_child_and_invalidated_before_authorization` 验证默认/覆盖、两子项冻结与一次确认。`runtime::fixed_model_protocol_dispatch_preserves_each_choice_and_identity` 验证请求、响应身份和用量。锁定 app-server 的两组实际出站请求见 `runtime-gpt-6-astra.json`、`runtime-gpt-6-sol.json`；提供方是本地脚本服务，没有调用线上模型。 |
| E06-B | `model_selection` 单元测试及 `runtime::model_identity_mismatch_and_corrupt_snapshots_fail_without_spending` 验证非法组合、作用域、能力版本、损坏快照、错误回执和进程启动前阻断。`protocol-fixture-a.json`、`protocol-fixture-b.json`保留未完成用量及预留；原 budget 回归继续验证未知用量不清零。 |
| E06-C | Runtime 从 execution revision 取冻结配置，每次 turn 明确发送原选择；第二组协议测试包含回答后继续。`project_hooks::hook_configuration_keeps_the_reviewed_model_override` 验证修复/验证扩展使用冻结选择，`multiple_repositories` 验证默认变更与真实作用域漂移。原恢复、暂停取消、撤权、重复及旧请求回归随完整后端采集执行。 |
| E06-D | 整组 review/authorize 返回冻结配置与预算，OpenAPI、生成类型和 models 查询为 #89 提供接口。完整手机交互与父组组合仍由 #89/#90 承接。 |
| E06-E | 原任务、累计预算和交付边界回归通过；HTTP 历史任务身份保持未知。测试边界分为脚本协议、真实锁定 app-server 加本地提供方、隔离 HTTP 数据 fixture；不将它们描述为线上付费模型或生产现场验收。 |

[身份证据索引](identity-evidence.json)记录原始文件路径和摘要。真实线上付费调用为零，
没有生产部署、队列放行或自动升级。本次缺少线上调用的适用授权及原账户有限预算，
因此不能据此宣布 E06 真实模型整链验收完成或关闭 issue；后续需补该边界的实际证据。

## 失败、恢复与时间

原始失败及恢复记录位于 `/home/gem/.local/share/codexsymphony/gh106-acceptance/`，
后续采集位于 `/home/gem/gh106-measurements/`。本目录的 `timings-*.jsonl` 是宿主阶段日志原样副本，
包含重叠阶段，不应直接相加为总耗时；并行前端采集与产物归档也不重复计费。

- source-01：SQL JSON 操作符优先级错误；加括号后修复，失败日志保留。
- source-02：覆盖率达标，三个函数 CRAP 略高于 10；拆分持久化和请求构造职责后重测。
- source-03：源码复查发现测试模块后还有新增函数，在编译早期主动终止；移动模块声明后重测。
- source-04：专用验证磁盘 ENOSPC；保留失败日志，将已结束尝试的产物移至另一磁盘并可逆压缩，
  source-05 在核对容量后换位置采集。`storage-recovery.json`记录恢复路径，未清理其他任务数据。
- source-05：完整后端采集和精确指标通过。frontend-02 是最终生成类型的前端测量。
- 秘密扫描首次因隔离 PATH 未含已安装工具目录、随后因报告目录只读而未完成；补齐调用环境和
  仅报告目录写入挂载后通过，没有更改宿主程序、锁或扫描策略。

产物压缩保留字节，可按存储恢复记录用 gunzip 恢复；旧尝试从未作为最终发布授权。

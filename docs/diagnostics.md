# 诊断保全与授权读取（GH-126）

遵循根 [AGENTS.md](../AGENTS.md) 的开发、测量与最终门禁程序。诊断只是非可信内容，不改变验证结论、候选身份、恢复决定、权限或预算。业务失败、执行失败和诊断可用性分别保存；原有验证代次与有界恢复继续拥有任务状态。

核心捕获明确登记的文件，不搜索任意目录。验证和附加交付 Hook 的合并输出保存为 `step-N.log`，批准的结构化检查使用 `--symphony-feedback-v2`、`protocol_version=2`，成功/失败/unknown 封装均含必填 `artifacts:[{kind,path}]`。生产者可通过 `SYMPHONY_DIAGNOSTIC_DIR` 写独立报告，协议输出保持 64 KiB 内的单个 JSON。生命周期 Hook 使用既有 `output_dir`；success/failed 均声明 artifacts。没有合法 JSON 时仍保全已存在的 stdout/stderr；监督器事实不伪造插件结果。

清单绑定 requirement/revision、源 Run、invocation/attempt、validation ID/代次、候选 commit/tree、配置/实现/环境摘要。phase 保留阶段和具体 step/hook 名称。尚未观察到的候选或实际环境明确为 null/unknown，不借用其他调用身份。artifact ID 是上述绑定及用途的 SHA-256，后续 attempt 或候选不会复用旧 ID。原始字节与脱敏 UTF-8 导出分别记录长度和摘要；export 摘要不能替代 raw 身份。

原始/导出内容与清单在同一个 PostgreSQL 事务入库，在工作区删除前完成。第一次已发布记录不可覆盖，包括 missing、expired；重启和重复调用只读原记录。内容写入失败会回滚清单，保留生产者原件并使调用返回错误；不会生成虚假 available。数据库故障需要按原失败恢复，不能无限重新执行生产者。

状态表达：available 表示实际文件已完整保留，不表示调用成功；partial 表示只保留原文 `[0,retained_bytes)`，原因说明输出/文件限额或停止。原总长度确实可见时记录 original_bytes，否则 null。missing 的 reason 区分未生成/不安全或采集失败，以及配额拒绝。expired 持久表示内容已删除，corrupt 表示导出摘要不符并拒绝内容。partial 导出的字节范围与原文不同；截断末行不导出，避免尚未完整捕获的敏感标记泄漏。

只支持 UTF-8 文本导出（无效字节替换），媒体类型按 JSON/Markdown/普通文本登记。压缩与二进制内容明确拒绝，既不解压也不下载外部 URL，因而不会处理压缩炸弹。超大单行 JSON 可作为独立文本报告分段读；它不作为放大的结构化结果封装。

Runtime 登记两个只读工具：

- `list_diagnostics({after:0})` 返回最多 8 个 artifacts 和下一页 cursor；next 为 null 时到末尾。
- `read_diagnostic({artifact_id,offset,limit})` 返回 artifact、offset/next/end、unit=`bytes` 和 text。limit 不超过 8192，offset 必须是 UTF-8 边界，不能容纳下个字符的请求拒绝。按 next 继续直到 end，重建导出并核对 export_bytes/export_sha256。

当前 Run 只读自己以及既有 repair_reservation 或恢复 session 明确关联的原失败内容，同需求的任意其他 Run 没有读取权。每次请求（包括重复 RPC）重验 Run 身份、thread/turn、暂停、取消、仓库撤权和作用域。读取历史报告还检查其生产者修订的仓库授权；新修订的有效权限不能重新开放已撤权的旧仓库报告。修复 Runtime 输入包含可用清单及读取方法，不要求宿主绝对路径；摘要或诊断文字不授予执行权。

已登录产品用户通过现有详情页查看清单、缺失原因及分段内容；下载由同一受保护接口逐段组成，核对完整导出摘要后创建文件。切换需求时丢弃旧请求结果。接口为 `GET /api/requirements/{id}/diagnostics/{after}` 和 `GET /api/requirements/{id}/diagnostic-artifacts/{artifact}/{offset}/{limit}`，200 JSON 契约见 [OpenAPI](../api/openapi.json)，授权/过期/错误范围统一返回 409 的安全说明。复用现有平台登录和同源请求保护；不暴露 raw payload 或任意文件服务。

配额复用已安装存储策略 entry_bytes、run_bytes、requirement_bytes、entry_count 和 hot.seconds。额外安全上限为单文件 1 MiB、单调用 16 MiB、单清单 64 项；原件加脱敏副本均计费，任务累计分配不会因过期或新 attempt 退款。没有部署存储策略的隔离 fixture 使用有界缺省值（1 MiB/8 MiB/32 MiB、256 项、一天），不构成运行就绪。全局/数据库实际容量及预留由现有存储测量校验。配额拒绝保留 missing 清单，不删除当时的生产者文件，也不承诺原文件能超越既有工作区清理期限；清单容量耗尽返回明确错误。过期复用现有存储消费者及未对账事实判定：活动 Run/组合验证、未完成验证与交付、预留修复、准备中的恢复、暂停/问答、未完成工作区操作，以及尚未确认停止的 Hook 均保护诊断。消费者完成且到期后才删除字节，保留身份、摘要、原因和累计分配；不会改变任务/Gate 状态。

文件采集使用既有 descriptor-bound 路径能力，拒绝绝对路径、`..`、URL、符号链接、非普通文件及跨挂载遍历。读取前后核对设备/inode/大小和时间戳，并核对可见路径仍指向同一对象。导出前完整脱敏，再分页；导出读取每次核对摘要。JSON 中的敏感字段按结构脱敏，包含跨行或嵌套的值；如有修改，导出为独立的规范化 JSON 字节并重新计算摘要。不完整或畸形的结构化内容若含敏感标记，整个导出保守脱敏，不输出无法可靠界定的字段值。诊断中的 HTML 在 UI 中按文本显示。

升级前用现有控制入口暂停并停止旧测试执行，确认进程停止，备份数据库与原证据目录，记录原任务累计消耗与未知预留；可归档旧任务。同步部署核心和受审 v2 生产者，应用新增表迁移，使用获授权的新测试任务验收。旧任务不转换、不重置额度。回退先停止新版并保存新证据，再恢复升级前备份与匹配的旧核心/生产者；不要让旧程序运行新格式数据库。备份/停机操作需另行授权，本次开发不执行部署、清理或付费模型调用。

验收记录将分别标明真实受控子进程、真实 Runtime 工具协议、HTTPS 浏览器 fixture 和外部模型/现场证据。源码测试入口为 `tests/diagnostics.rs`、`tests/runtime.rs::real_runtime_stdio_client_reads_entire_retained_diagnostics_and_verifies_digest` 及 `tests/unit/diagnostic_capture.rs`；完整 Gate 结果和 AC 对照在任务证据中保留，不以测试名称或清单勾选代替 PASS。

# 自然语言生成与编辑草稿（M1-02）

需求工作台的“导入与查看父子草稿”入口同时提供自然语言生成。填写来源说明和需求后，
显式点击“生成草稿”；刷新生成记录查看结果，再选择“打开生成结果（替换编辑区）”。
生成记录可展开查看输入；失败或冲突时可查看未应用的模型输出。
在原文编辑区修改目标、AC、验证计划，删除子项或调整 `order` / `depends_on`，保存后重新打开。
排序只改变显示顺序，稳定 ID 不重建。来源、原始生成输入/输出和各版本分别保留。

生成使用 GH-59 的 `codexsymphony-draft/v1` 和相同持久模型；小需求可以只有一个子项，
大需求可以包含多个 `code_change` 和 `validation_only`。所有结果仍是未评审 Draft。
模型不得确认缺失事实：提示要求未知字段内容留空、未知仓库用 null；服务端显示待补齐项。
结构校验不证明语义正确，用户仍需评审模型建议。后续 M1 已开放[覆盖映射与整组授权](group-review.md)及[依赖队列](group-queue.md)。草稿生成本身不授权执行；validation_only 实际执行和最终业务验收仍未实现。

## 调用与记录

只有 `POST /api/draft-generations` 可能调用模型。导入、编辑、列表及读取不调用。
请求体严格包含 `request_id`、`draft_id`（新建时 null）、`version`（新建时 0）、`label`、`text`。
来源说明限制 512 字节，输入限制 16000 字节。`GET /api/draft-generations` 和
`GET /api/draft-generations/{id}` 读回独立记录，包含输入、输入版本、输出版本、状态、错误、
实际用量及 Runtime thread/turn 标识、实际 user-agent 和非秘密设置摘要。

一条生成记录最多启动一个 turn，最多 120 秒、30000 token；部署可缩短超时。
收到超限通知即终止进程，不增加下一 turn。通知可能滞后，不能把它宣传为精确计费上限。
用量使用现有 `budget::Usage`，缓存输入是输入子集，不重复相加；缺失数据保持未知。
生成用量与编码执行预算、AgentRun 和全局代码执行 owner 分开，不产生 PR。

`request_id` 重用不同输入返回 409；相同输入指纹即使换 request_id，也返回原记录。
失败或中断的同一请求不会自动重试。修改输入后可以显式申请新生成；没有后台重试循环。
同一时刻最多一条生成记录运行，但不会抢占或释放代码执行 owner。
配置未就绪时返回 503；已存在记录仍可幂等读回。

完成时与 Draft / revision 在一个事务内提交，使用请求绑定的输入版本做 CAS。
生成期间的已保存编辑优先保留；冲突的生成输出保存在生成记录中，不能覆盖用户修改。
畸形 JSON、未知字段、非法 kind、重复 ID、悬空/循环依赖及未登记仓库由同一确定性校验拒绝，
失败不产生部分 Draft。未知必填结构字段报错；允许为空的内容显示待补齐。
前端刷新生成记录不会自动替换正在编辑的原文，网络失败会保留输入和请求 ID。

服务启动在现有单实例锁内将未结束记录标为 `interrupted`，不自动重新调用。
生成进程设置 Linux parent-death signal，API 退出时不遗留继续运行的模型进程。
已成功观测并写入的用量持久化；中断标记用量不完整，无法确认的远端消耗不按零计算。
若数据库故障使终态无法写入，保留原运行意图，恢复数据库后重启服务将其标为中断；不重放模型。

## 部署配置

`DRAFT_GENERATION_CONFIG` 指向操作员管理的 JSON 文件，例如：

```json
{
  "codex_home": "/absolute/operator-provisioned/runtime-home",
  "model": "gpt-6-astra",
  "timeout_seconds": 120
}
```

复用锁定 Codex 0.154.0、现有 RPC Transport 和操作员配置的 Runtime home/provider/authentication。
API 和模型输出不能指定可执行文件、配置路径、仓库权限或凭据。
生成以独立临时 cwd、read-only、禁止审批、关闭 shell/apply-patch/web/multi-agent 工具的方式运行，
没有平台编码动态工具；出现工具/审批 RPC 即失败。部署 home 必须由操作员管理，不能由导入文本构造。
本项没有修改生产部署或生产配置。缺少配置时导入及编辑仍可用。

迁移 `0018_draft_generation.sql` 增加独立生成表，不修改旧需求、Run、预算或 owner。
HTTP 契约见 `api/openapi.json`；验收及真实调用与脚本化测试的区分见
[GH-61 验证映射](quality/gh61/README.md)。

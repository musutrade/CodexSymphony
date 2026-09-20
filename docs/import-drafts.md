# M1-01：导入父子 Draft

入口：需求工作台或需求列表 → **导入与查看父子草稿**。粘贴原文，选择格式，填写来源说明后保存。
已保存草稿可打开、修改整组原文并保存新版本。目标、范围、父 AC ID、来源、子项顺序、依赖、仓库、AC 和验证计划均可查看。

## 输入契约 v1

可直接使用 [JSON 样例](../apps/server/tests/fixtures/draft-v1.json) 或 [Markdown 样例](../apps/server/tests/fixtures/draft-v1.md)。
样例的 `repository_id: 1` 是演示值，须改为需求工作台已登记仓库的内部 ID；尚未选择时明确填写 `null`。
不登记第二套仓库身份或认证。样例不会自动保存或授权。

结构化输入为 JSON 对象：

- `schema` 固定 `codexsymphony-draft/v1`。
- `parent`：`id`、`goal`、`scope`、`acceptance_criteria`（`id`、`description`）。
- `children`：每项包含 `id`、`parent_id`、`kind`、`order`、`depends_on`、`repository_id`、`goal`、`acceptance_criteria`、`validation_plan`。
- `kind` 仅 `code_change` / `validation_only`；`order` 为非负整数且组内唯一。显示按此顺序排序，原输入数组顺序不被改写。
- `depends_on` 是同组子项 ID 数组；不接受重复、悬空、自引用或环。当前不检查拓扑顺序与 order 是否一致，整组 Ready 评审留待后项。
- `validation_plan` 为自由文本，仅存储，不解释为可执行命令。

除 `repository_id` 可省略（读回为 null，并提示待选择仓库）之外，所有键必须显式提供；拒绝未知键。目标、范围、来源说明、AC 描述和验证计划可填空字符串，AC/子项可填空数组，仓库可填 null；这些情况显示待补齐提示，不生成默认答案。
结构标识不可为空，限 80 字节 ASCII 字母、数字、`-`、`_`。父/子 ID 在组内唯一，AC ID 在各自父项/子项内唯一。
这些 ID 由输入指定，读回与更新不自动重编号；编辑 ID 即显式改变引用，所有依赖必须同步修正。
组的服务器身份为 `draft-<UUID>`，子项完整身份是该组身份与本地子项 ID 的组合。不同草稿组可采用相同模板 ID，不与旧整数需求 ID 混用。

Markdown 是可确定解析的固定包装，使用 LF 换行：首行 `# CodexSymphony Draft v1`，空行，单个标记为 `json` 的围栏代码块，其内容就是上述 JSON 对象。
外围仅允许空白，不推断标题、列表或任意自然语言。格式版本不匹配、非法 JSON、缺键等错误明确拒绝。原文限 256 KiB，来源说明限 1024 字节，子项最多 200 个。

## API 与版本

- `GET /api/drafts`：草稿组列表。
- `POST /api/drafts`：`{"version":0,"source":{"format":"json 或 markdown","label":"来源说明","text":"完整原文"}}`。
- `GET /api/drafts/{id}`：完整草稿、当前来源、SHA-256、版本及待补齐提示。
- `PUT /api/drafts/{id}`：相同写入结构，version 必须为刚读到的当前版本；成功版本加一。

POST/PUT 成功为 200；版本冲突 409、非法内容 422、对象不存在 404。数据库不可用为 503。
所有路由沿用 Host/Origin/CSRF 防护。版本冲突保留浏览器输入，先复制输入再重新打开最新版本合并；不会自动覆盖另一窗口的修改。
创建请求没有跨请求幂等键；响应丢失时先刷新草稿列表核对来源，避免重复导入。

数据库迁移 `0017_import_drafts.sql` 随正常启动自动执行，可用于已运行到 0016 的数据库。
新增 `imported_draft` 和不可覆写的 `imported_draft_revision` 历史表，旧单需求、修订、预算和队列记录不迁移或改写。
父子对象一起保存在版本化 JSONB 文档，来源以原始 text、format、label 保存；SHA-256 对原文 UTF-8 字节计算。
更新锁定当前行，执行版本校验，当前内容及历史版本在同一 PostgreSQL 事务提交。失败不留下半组数据。
历史来源保存在每个版本中，当前界面和 API 显示最新版本；历史版本浏览界面不在本项范围。

## 授权与尚未实现的能力

导入草稿没有 Ready 状态，不写入旧可执行 requirement 表。旧 Ready/控制路径收到 `draft-…` 身份会返回明确的 409。
父项不占队列。导入、读取、编辑不创建 Run、模型调用、PR 或预算授权；严格跨仓串行保持原流程。
来源文本无论包含什么命令或授权措辞，都只按需求资料保存，并以文本显示。

未实现：模型生成、自然语言拆分、父子 AC 覆盖映射、原子整组授权、依赖调度、validation_only 实际执行、自动合并、M2/M3。
本项不代表 B01、完整 0a 或日常 V1 验收通过。A13 缺少的四份历史原件仍按原记录保持缺失。

测试与证据映射见 [GH-59 验证记录](quality/gh59/README.md)。

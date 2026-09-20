# 未开始队列编辑与变化部分评审（M1-05）

从父子草稿的整组评审页进入“编辑未开始队列”。电脑、手机及键盘使用同一持久队列。
上下移提交完整顺序及当前队列版本；合法重排只增加队列版本和审计记录，不改写输入、内容授权或预算。
依赖必须先于后继，重复、缺失 ID 或已领取项的位置变动会被拒绝。

内容编辑器预填需求与评审 JSON。保留已有子项 ID，可新增、移除或修改尚未开始项的目标、AC、验证计划、依赖、修复范围及预算。
修改仓库策略仍使用仓库配置入口；评审需绑定其最新策略版本。
保存后展示变化前后完整内容、精确 revision、仓库策略和预算，以及冻结的受影响项和依赖闭包。
保存不构成授权。用户核对后点击“确认以上变化版本与策略并重新授权”；父 AC 缺覆盖、策略过期或余额不足时拒绝，仍保留待评审版本。

未受影响子项继续使用原授权及不可变输入；已完成事实保持原版本和证据绑定。
共享父范围、父 AC、完整链 AC 或显式组预算变化会影响整组。子项内容、AC、依赖、覆盖映射、策略版本或预算变化影响该项和变化前后依赖闭包（包含调度器绑定的同仓前序基线依赖）。
只替换待评审方案时，按当前已授权内容重新计算闭包；不会把一次未授权编辑当作新的授权基线。

已准备不可变 initial Run 输入、已领取、已完成或取消的项不能通过此入口伪装成未开始。
运行项需使用既有停止/暂停、工作保全、外部动作核对及关联新 Draft 的重新评审流程；本入口不会修改运行快照或隐式恢复。
用户暂停独立保存，确认差异不会解除暂停。取消不代表 Done，也不能满足后继依赖。
同一子项不能换 kind 或仓库身份；需要不同身份时新增明确评审的子项，原账本继续保留。
移除保留旧执行身份和累计账本，不删除用量、不重置预留，重新加入相同 ID 复用原身份。

## API 与事务

`POST /api/drafts/{id}/queue-edit` 使用原 Host/Origin/CSRF 控制，提交：

```json
{"request_id":"unique-operation","version":1,"change":{"kind":"reorder","order":["C1","C3","C2","C4"]}}
```

- `reorder`：`order` 必须是当前所有子项的排列。存在待评审修改时拒绝。
- `propose`：提供 `document` 与 `review`。`review.parent_revision` 为当前 Draft revision + 1，子项和覆盖映射在确认时检查对应版本。允许保存覆盖尚不完整的方案，确认时必须完整有效。
- `approve`：只提供 `edit_version`，授权数据库中该精确方案和策略快照，不接受客户端另带的未展示内容。

成功返回 `{version, affected}`。相同 request_id 和相同完整输入返回原结果；同 key 不同输入、过期队列/编辑版本和领取竞争返回 409。非法顺序、覆盖或预算返回 422。
失败不部分更新授权、需求 revision 或额度。503 表示事务未能提交，可用同 request_id 重试。
`GET /api/drafts/{id}/review` 返回当前顺序与 `pending_edit`（版本、内容、评审、影响集合、仓库快照）。
纯重排的版本属于队列，不增加 Draft/授权 revision。

迁移 `0021_group_edits.sql` 保留已有队列和授权：顺序从既有输入初始化，新冻结/移除标记默认为 false。
编辑、投影和领取共用 advisory transaction lock 13002。队列 CAS、冻结、授权、需求新 revision、预算限额及审计都在事务中提交。
`group_execution_item` 的稳定 Requirement 身份不变；`requirement_revision`、原授权、完成事实及所有用量/预留历史不改写。
队列顺序单独存储，领取时以最新合法顺序绑定同仓前序完成事实。

## AC 与验证

| AC | 真实验证 |
| --- | --- |
| AC01 | `group_edits::reorder_restarts_without_reauthorizing_and_content_edits_keep_completed_facts`：顺序重启、版本、幂等、过期与非法依赖；领域排列拒绝测试；实际 API 进程重启证据 `artifacts/gh64/api/restart.json` |
| AC02 | `delta_review_is_atomic_idempotent_and_preserves_identity_and_balances`：仅冻结 C2/C3/C4、保留 C1；`invalid_coverage_budget_and_claimed_inputs_never_partially_authorize`：缺覆盖拒绝；完成事实持久化夹具保持 |
| AC03 | 新增/移除与策略漂移/预算不足测试；授权写入 PostgreSQL trigger 注入故障回滚；重复确认不重复授权；累计已用/预留及稳定身份保持 |
| AC04 | `group_queue::queue_edit_and_real_claim_serialize_in_both_orders`：真实数据库锁竞争及 Runtime planner/reserve，两种胜出顺序；既有 Runtime、execution/workspace 停止保全与正确版本恢复回归 |
| AC05 | `queue-edit.spec.ts`、真实 API Playwright `group-review.spec.ts`：桌面/Pixel 7、上下移、内容编辑、持久差异与明确评审、冲突、键盘、axe、减少动效、强制颜色；旧单需求与暂停/取消回归 |

完成事实测试使用明确标记的合成协议/持久化夹具，不代表 M3 实际业务验收。
validation_only 执行、自动合并、父级最终业务验收仍明确等待。历史 A13 原件缺失记录保持不变，见[原恢复索引](quality/gh25/evidence-recovery.md)。
实际命令、结果及限制见 [GH-64 验证记录](quality/gh64/README.md)。精确发布提交双 Gate 由独立宿主验证。

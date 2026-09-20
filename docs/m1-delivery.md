# M1 需求与队列使用、迁移及交付索引

M1 提供需求录入、父子草稿、覆盖评审、整组授权、严格串行依赖队列和未开始项变更评审。
这不是日常 V1 全链路交付：M3 的自动合并、适用子项业务验收、validation_only 实际执行和父级最终验收尚未实现。
Submitted、CI PASS、已合并观察与业务 Done 分开保存；没有适用完成证据时明确等待。

## 使用

1. 在需求工作台选择已登记仓库。小型旧单需求仍可创建、编辑、评审 Ready、撤回或使用原暂停/取消控制。
2. 打开“导入与查看父子草稿”。自然语言填写来源和需求后显式生成；生成结果仍是 Draft。
   也可粘贴 `codexsymphony-draft/v1` JSON 或规定格式的 Markdown。导入不调用模型，不产生执行授权。
   格式与例子见[导入说明](import-drafts.md)，生成配置、幂等与失败语义见[生成说明](draft-generation.md)。
3. 打开结果并修改目标、范围、AC、验证计划、子项和依赖。保存后重开核对；稳定 ID 和历史版本保留。
   大需求拆为可在前置合并基线上独立验证的 code_change，最后的 validation_only 承担集成条件。
4. 进入整组评审，逐条核对父 AC → 子项 AC → 验证步骤；完整链条件映射到最后集成项。
   检查精确 revision、仓库策略、允许修复范围、机器检查选择器和预算。存在映射不等于语义覆盖。
   保存评审版本后一次确认整组授权。缺覆盖、非法依赖或版本过期时整组拒绝，无部分 Ready。
5. 队列显示全局 owner、顺序、依赖、等待原因与完成计数。电脑和手机读取同一个 API/数据库；刷新不会重新授权。
   M1 不承诺整组能自动执行至 Done。validation_only 不创建空 Run/PR，父项等待最终验收。
6. 未开始项可合法重排；内容/范围/AC/预算变化先冻结受影响项及其依赖，再评审变化部分。
   版本冲突时保留本地输入，重新读取并核对差异。运行快照不可通过这个入口修改。
   暂停仍需显式恢复；取消不满足后继依赖。已用额度和在途预留不会因编辑、移除重加或重启清零。

详细契约：[整组评审](group-review.md)、[依赖队列](group-queue.md)、[变化评审](queue-editing.md)、
[执行控制](execution-control.md)、[预算](budgets.md)。API 请求和响应以 `api/openapi.json` 为准，
本次未新增或改变接口。测试中的用户点击只授权一次性测试数据，不代替真实产品用户评审。

## 从 GH-59 升级与恢复

GH-59 的迁移终点是 `0017_import_drafts.sql`；M1 后续依次应用 0018（生成记录）、0019（整组评审）、
0020（执行投影/精确完成事实）和 0021（队列版本/变化评审）。由正常 API 启动的 SQLx migrator 执行，
不手工修改 `_sqlx_migrations`、旧 Contract、owner、预算、Run 或来源历史。

操作员应先备份数据库及配置/工作保全证据，停止原 API 并确认旧执行组静止，使用经精确提交双 Gate 验证的版本启动。
升级失败先保留日志和数据库事实，修复具体迁移问题；不删除表或更改迁移摘要来掩盖失败。
旧进程身份不明、保全未完成、外部动作结果未知时维持阻塞并对账；不能通过清空 owner 强行恢复。
本 Issue 只在独立测试数据库验证升级和重启，没有部署生产。

重启保留父子 Draft、来源/版本、授权、队列、预算与暂停意图；未完成生成标为 interrupted，不自动重放模型。
收到过期版本时重新读取用户数据后再评审，不用旧请求覆盖新内容。恢复已有 Run 沿用原额度和不可变输入。
需要回退版本时由操作员从一致性备份制定恢复方案，不对已升级业务数据直接执行破坏性 down migration。

## Issue 与 AC 对应的实际测试

本表列出可定位的测试入口；实际运行结果、源码身份、原件路径与 SHA-256 由
[GH-65 验证记录](quality/gh65/README.md)和 `artifacts/gh65/` 索引记录。测试尚未执行或失败时不能只凭本表宣称通过。

| 范围 | GH-65 AC | 测试入口及证据内容 |
|---|---|---|
| GH-59 / M1-01 | AC01、03、05 | `tests/drafts.rs` 的 parser、原子导入、旧单需求拒绝混用及真实 API 重启；新增 `gh59_database_upgrades_without_rewriting_draft_history_or_execution_facts` 从 0017 保存的数据升级；`e2e/drafts.spec.ts` 在桌面/手机通过真实 API 导入 Markdown/JSON |
| GH-61 / M1-02 | AC01、03 | `tests/generation.rs` 的有界失败、事务/CAS、恢复和用量；`e2e/generation.spec.ts` 的显式真实模型模式保存原始生成记录、小需求、大需求及修改结果；默认协议 fixture 明确不冒充真实模型 |
| GH-62 / M1-03 | AC01、03 | `tests/group_review.rs` 覆盖、依赖、过期策略/版本、故障事务全组拒绝；`e2e/group-review.spec.ts` 键盘保存和一次授权；`e2e/m1-real-flow.ts` 将真实大需求接入覆盖评审和同数据跨设备读回 |
| GH-63 / M1-04 | AC02、04 | `tests/group_queue.rs` 的 `global_claim_restart_budget_and_completion_protocol`、跨仓/旧单需求共享 owner、精确版本及等待原因；外部完成 Fact 为显式模拟，非 M3 自动交付证据 |
| GH-64 / M1-05 | AC02、03、05 | `tests/group_edits.rs` 的原子差异评审、预算保留、非法覆盖/策略漂移拒绝；`tests/group_queue.rs` 的编辑与实际领取双方获锁顺序；`e2e/group-review.spec.ts` 的重排、变化评审与冲突展示 |
| GH-65 / M1-06 | AC01–06 | 真实生成连续链路、GH-59 升级、API 重启读回、Rust/前端全套适用检查、本说明和逐文件证据清单；精确 PR 双 Gate 由独立宿主执行 |
| 旧单需求与运行回归 | AC02、03、05 | `requirements`、`execution`、`runtime`、`runtime_real`、`budget`、`delivery`、`workspaces` 等完整 Cargo tests；`e2e/requirements.spec.ts` 与 `operations.spec.ts` |

## 验收边界与后续

仅按日常 V1 的 B01（入口/评审）、B02（拆分/覆盖/队列）、B07（手机接续/控制）的已执行子场景记录。
不把这些子场景提升为完整 B01、B02、B07，更不宣布 B01–B08 或整个 V1 通过。
数据库注入的合并/完成事实和脚本化 provider 用于受控协议验证，必须与真实模型、浏览器和 Runtime 原件分开标记。

M2：登录、公网部署、通知。M3：自动合并、CI/合并后关联修复、validation_only 实际执行、
绑定精确版本的最终业务验收及父项完成。M1 不提前建设通用 Planner 或完整工作流框架。
历史 A13 四份缺失原件仍按[恢复记录](quality/gh25/evidence-recovery.md)列为 unresolved：
`repositories.json`、`first-operations.json`、`second-operations.json`、`second-operations-latest.json`。
本次新证据不补写旧摘要、不替代缺失原件，也不重新宣布 Phase 0a 发布通过。

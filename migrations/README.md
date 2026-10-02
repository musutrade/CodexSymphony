# Database migrations

The server runs SQLx migrations at startup before accepting HTTP requests.
The initial skeleton contains no business tables yet. Add versioned SQL files
here with the first domain persistence task; never modify an applied migration.
Integration tests require a dedicated TEST_DATABASE_URL and never infer a production URL.

0017 新增父子导入草稿与版本来源历史表，旧单需求和执行表不变。输入及迁移边界见 `docs/import-drafts.md`。

`0018_draft_generation.sql` adds independent advisory generation intents/results and a single active-generation constraint. It does not change Requirement/AgentRun ownership or budgets; failed generation never partially inserts an imported Draft.

- `0020_group_execution.sql`: stable authorized child-to-Requirement projection, immutable dependency inputs/completion receipts, shared ordered queue view, cumulative group accounting attribution. Parents and validation-only children never receive a coding Requirement/Run.

- `0029_linked_repair.sql`: original-item post-merge/integration repair provenance, shared reservations and immutable Run repository inputs. Explicit scope opt-in, migration compatibility and rollback limits: [linked repair](../docs/linked-repair.md).
- `0030_cancelled_group_queue.sql`: exclude fully cleaned cancelled children from execution slots; retain their history and require real completion for dependent children.

0032 adds append-only `environment_observation` for optional reviewed repository
plans. Existing authorization, AgentRun and budget rows are untouched. Keep this
table during rollback; tasks with a frozen environment plan require a compatible
binary. See `docs/repository-environments.md` for upgrade/recovery conditions.
The same migration observes validation and delivery transitions with nullable
timestamps. Historical starts are not backfilled; unavailable durations stay unknown.

`0043_integration_diagnostics.sql` extends diagnostic read admission to the
started repair reservation linked to an exact integration invocation. Existing
Run/session admission remains unchanged; accounts, budgets, reservations and
delivery rows are untouched. Keep the authorization functions and retained evidence during
rollback; see [diagnostics](../docs/diagnostics.md) for the capture and upgrade boundary.

0043 同时为产品和 Runtime 诊断读取核验原集成冻结集合中全部仓库的历史授权；次要仓库撤权和撤权后重新授权均不能开放旧报告。回退兼容边界见诊断文档。

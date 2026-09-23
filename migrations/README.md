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

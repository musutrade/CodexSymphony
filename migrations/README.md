# Database migrations

The server runs SQLx migrations at startup before accepting HTTP requests.
The initial skeleton contains no business tables yet. Add versioned SQL files
here with the first domain persistence task; never modify an applied migration.
Integration tests require a dedicated TEST_DATABASE_URL and never infer a production URL.

0017 新增父子导入草稿与版本来源历史表，旧单需求和执行表不变。输入及迁移边界见 `docs/import-drafts.md`。

`0018_draft_generation.sql` adds independent advisory generation intents/results and a single active-generation constraint. It does not change Requirement/AgentRun ownership or budgets; failed generation never partially inserts an imported Draft.

# CodexSymphony

Personal AI Software Factory 的设计规格与实施前验证。目标：**评审通过的需求在正常路径上零介入地变成已合并代码**——人只做需求评审和处理异常，Agent 编码，平台负责调度、验证、合并与交接。

## 仓库内容

| 路径 | 内容 |
|---|---|
| `Personal_AI_Software_Factory_综合方案.md` | 综合方案 V9.0。第 0 章是前提与分期，第 23 章是实施队列，第 24 章是 V1 交付门槛；其余章节是各子系统的规格 |
| `docs/architecture-boundaries.md` | **规范性架构边界**：三个真相分离、Harness-Gate 仅作为 Validation Evidence、禁止状态坍缩与职责越界 |
| `spikes/s1/` | Codex app-server `dynamicTools` 与 workspace-write 沙箱边界（Python，真实模型运行） |
| `spikes/s2/` | GitHub App：installation token → 条件 push → PR → Checks → sha 守卫合并（Python） |
| `spikes/s3/` | Cloudflare Access JWT 在 Axum 中的验证、Tunnel 源站隔离、会话撤销（Rust） |

每个 spike 目录的 `README.md` 有结论表、对方案的修正、复现步骤和未验证项；结论已回写到方案 23.S 与对应章节。

## 方案要点

- **一个人工决策点**：`POST /ready` = 评审通过。评审通过的标准是每条验收标准都可机器验证。
- **人工介入只是异常中断**：Agent 提问、沙箱外请求、安全门禁失败、修复预算耗尽、评审后输入被改。待办箱为空是正常状态。
- **三个真相分离**：Requirement 状态是业务真相，AgentRun 是执行事实，GitHub / CI 是外部观察事实。
- **Harness-Gate Result = Validation Evidence，不是第四个状态源**：Harness-Gate 可对精确 source/config/evidence identity 给出权威门禁判定，但 CodexSymphony 只把它作为编排证据消费；它不得直接把 Requirement 置为 Done，也不得把 AgentRun 改写为失败。完整规则见 [`docs/architecture-boundaries.md`](docs/architecture-boundaries.md)。
- **Agent 不碰 Git 远端**：只在 worktree 内改文件，通过受控工具 `create_local_commit` / `report_completion` 请求提交与声明完成；push、PR、合并由平台经 outbox 幂等执行。
- **分期**：0a 本机闭环（需求 → PR）→ 0b 手机可用 → 0c 加固 → Phase 1 自动合并与自动 Done → Phase 2 描述生成 Contract、多仓。

## 已由 spike 确定的硬约束

- 每个 Run 单独启动一个 `codex app-server`，进程 cwd = 该 Run 的 worktree；否则 `.git` 不受沙箱保护。
- workspace-write 沙箱不限制读：0a 只接管自己可信的仓库，0c 的独立 UID executor 是接管其他代码的前提。
- GitHub App 最小权限 `contents: write` + `pull_requests: write` + `metadata: read`。
- Access JWT：`owner_id` 绑 `sub`；撤销只在边缘生效，源站必须只能经 Tunnel 到达。
- `jsonwebtoken 11` 必须开 `rust_crypto` feature。

## 状态

设计规格 + spike 阶段，尚无平台代码。下一步：S4（Harness-Gate 在本项目上的耗时/误报）、S5（真实需求下沙箱外请求失败率）、S2 补测（真实 workflow 的 check-run 名称与 `mergeable_state` 序列），然后进入 Phase 0a。

## 参考

- [OpenAI Symphony](https://github.com/openai/symphony) — 执行内核的设计来源
- [Harness-Gate](https://github.com/musutrade/Harness-Gate) — 本项目的确定性质量门禁

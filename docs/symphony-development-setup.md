# 用现有 Symphony 开发 CodexSymphony

`WORKFLOW.lifecycle.md` 是本机 Elixir Symphony lifecycle 扩展的开发配置。
它不是未来 Rust 平台读取的 Repository `WORKFLOW.md`，也不表示服务已经启动。

## 启用前必须完成

- 将本工作流、版本锁及实施所需规格提交到开发仓库，否则 clone 无法取得这些输入。
- 提交并实际运行 `.github/workflows/quality.yml`，其 job 名为 `Harness-Gate`，已写入
  `lifecycle_required_checks`。在 GitHub 保护规则中将其设为必需并验收；本次未修改远端规则。
  门禁配置与源码骨架已经落地；本机受信宿主及三路插件已完成完整隔离 PASS；远端受信输入供给仍待接通，详见
  `.harness-gate/QUALITY.md`。必须取得真实完整 PASS 后才能自动交付。
- 为此仓库准备独立的 `CODEXSYMPHONY_WORKSPACE_ROOT` 和受限服务环境中的
  `GITHUB_TOKEN`；控制器需要查询 Issue/PR/Checks/Review 及交付所需权限。
  实施任务必须已评审并带 `symphony-ready`，依赖按顺序准备；不要批量标记未评审任务。
- 确认实际会话注入 `github_api` 动态工具，并验证宿主 GitHub 授权。
  本机 `Codex.DynamicTool` 将调用交给 `GitHub.AgentTool`，由宿主侧执行 REST 请求；
  通过 Git Data API 发布 blob/tree/commit/ref 并创建 PR，无需 Agent 执行 `git push`。
  默认 `.git` 写保护保留，不需要扩大可写根。沙箱的依赖下载网络与宿主 GitHub 请求
  是不同执行路径，分别验证。
- 验证开发执行器所需网络、模型认证、锁定的 Codex 版本和任务特定依赖。
  主机网络策略影响同机其他 Run，不得为此工作流直接覆盖现有策略。
  工作流固定使用工作区内 `target`，避免依赖外部 Cargo 目录写权限。

## 部署边界

使用 `/home/gem/symphony/elixir` 的本地 lifecycle 扩展及其 schema 校验配置。
采用独立服务、端口、日志与工作区；同一仓库只能有一个控制器。
部署时将已审定工作流复制到 Agent 工作区之外的服务状态目录，再将绝对路径传给控制器。
交接台账生成在 `<workflow-path>.handoffs.json`，目录必须可由服务写入，且不能由 Agent 写入。
不要复用 Harness-Gate 的台账或将其历史恢复特例复制到本项目。

默认串行交付、最多 3 次 Run、1 次 CI 修复、4 小时累计活跃运行、1 亿累计 token
（含缓存输入；不是费用估算）。这些是开发控制器的初始运行限制，按实际运行复评；
与综合方案中未来 Rust 平台的分阶段预算分开。等待 CI 不占模型时间，重启不重置计数。

完成上述启用项后，再执行真实沙箱和一次测试 Issue 的端到端验收。
离线解析通过只能证明配置和模板格式正确，不能证明 GitHub 交付或业务验收通过。

## 当前交付实现的依据

本次核对本机 `/home/gem/symphony/elixir/lib/symphony_elixir/` 下的
`codex/app_server.ex`、`codex/dynamic_tool.ex`、`github/agent_tool.ex` 和
`github/client.ex`：Agent 发起 `github_api` 调用，宿主用绑定的 Tracker 配置执行请求；
声明仍需要已创建 PR 的编号、分支和真实远端 SHA，随后控制器对账 CI、合并与关闭 Issue。
因此不能改成只写“本地完成”就期待控制器自动生成 PR 的未实现协议。

Harness-Gate 的现存 WORKFLOW 文本仍使用 “final push” 措辞，但其交接台账
GH-182 的 validation_summary 记录了 `.git` 只读时通过 Git blob/tree/commit/ref API
发布并核验 PR head 的实际路径。本项目以该路径和源码为依据，不照抄旧措辞或特殊可写根。

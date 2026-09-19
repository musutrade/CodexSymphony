# GitHub App 预检与外部观察（GH-16）

GH-24 集成支持 Actions 源显式配置 `branch_from_pr: true`，将检查绑定到当前同仓 PR
的 head ref；未配置或 false 仍使用原精确 `branch`。该选项须经受信配置版本审核，
不自动迁移旧策略。仓库 ID、workflow ID/blob SHA、App、event、精确 head SHA 及
suite/job/attempt 仍全部核对；fork 或缺少 head ref 拒绝。历史探针成功不替代新 PR 的证据。

当前 0a 只预检准备、发布与只读观察。控制面按 repository ID 请求短期 installation token：
Contents / Pull requests write，Checks / Actions read；不请求 rerun、merge 或 Checks write。
JWT 使用 RS256，回拨 60 秒，5 分钟有效；token 在到期前 60 秒刷新，缓存最多 5 分钟。
401 只刷新重试一次。重定向关闭，凭证只驻留控制面内存，不进入数据库、日志、Run 或 Agent 环境。
0a 同 UID 的读取限制仍适用；生产配置及私钥由操作者放在 Agent 工作区之外。

## 控制面配置和只读验证

配置 JSON 由操作者创建在工作区外，包含 `app_id`、`private_key_path`、`policy`、`probe_pr`。
`policy` 包含已登记的 `repository_id`、`repository`（owner/repo）、`default_branch`、
`version`（与登记仓库策略版本相同）、`wait_seconds` 和 `required` 数组。
每个 selector 有 `name` 与 `source`：

- `{"kind":"actions","app_id":15368,"workflow_id":实际ID,"workflow_sha":"受审 workflow blob SHA","event":"pull_request","branch":"实际 PR head 分支"}`
- `{"kind":"check_run","app_id":受信 App ID}`
- `{"kind":"status","creator_id":受信发布者用户 ID}`

`probe_pr` 必须是该仓库已有的授权只读验证 PR，预检不会创建它。workflow 的默认分支和被观察 PR head 的 blob
必须与策略固定值一致，workflow 必须 active；已有精确 head 的 run/job 提供来源及触发事实。
检查结果可以失败，但来源和当前有效运行必须能解析。
Status/普通 App check 的外部触发配置尚未接入可验证适配器：观察照常解析，但就绪预检明确阻断，
不会拿历史成功冒充已验证的自动触发。当前可放行的是固定 GitHub Actions workflow 的 head 检查。未配置的来源、同逻辑身份冲突和缺失结果会阻断。
当前选择 PR head，不能把 GitHub Actions 的 head_sha 当作实际 checkout SHA；test-merge 验证
与实际 checkout 的可信证明由后续验证集成提供，此处不声明验证 PASS。

```sh
cargo run --locked -p codexsymphony-server -- --github-inspect /absolute/control-plane/github.json
```

这是无数据库、无模型、无 PR/rerun 写入的产品适配器入口；唯一 POST 是按仓库申请 token。
输出脱敏能力及原始来源身份，缺能力/访问失败退出非零。默认 API 固定为 `https://api.github.com/`；
`api_url` 仅允许该地址或 fixture 的 `127.0.0.1`，不能将 App 凭证发送到任意远端。
不得把真实私钥或 token 放入命令参数、仓库或报告。

服务配置 `GITHUB_APP_CONFIG` 为同一外部 JSON 路径即可启用后台读取。先通过现有仓库登记流程
保存精确仓库身份和策略版本；GitHub 配置不会自行登记或修改 Requirement。未提供配置时
API 照常启动，GitHub 能力未就绪，Ready 不领取。

## 记录与消费边界

- `github_repository` 保存策略、所需权限、实际权限、workflow/rules/来源证据、预检时间与缺项。
  `github_repository_capability` 动态计算过期与仓库授权版本变化后的 stale。
- 内部 `github_store::link` 校验 Requirement 评审快照中的仓库后关联 PR，供后续 outbox 调用。
  `github_pr_observation` 展示 PR/head/base/ref、合并事实、Checks/status 身份、最后同步时间、
  stale 和 HTTP 错误/尝试次数/next_attempt_at；故障保留上次成功快照，不能作为新事实。
- 每个关联 PR 和仓库能力 60 秒刷新，失败等待 120、240、300 秒，此后保持 300 秒；成功恢复 60 秒。
  轮询任务不调用模型。数据库故障不会启动模型或写 GitHub。
- 在同一个领取事务中检查当前仓库/评审/能力策略版本、60 秒新鲜度、stale 和缺项，缺能力时
  不创建 AgentRun。Runtime、实际远端交接仍未接通，正常后台仍不启动编码。
- PR 读取前后身份必须一致。`merged=true` 或非空可靠 `merged_at` 才确认合并，缺字段为未知；
  closed、非空 test merge SHA、403/404 均不能确认合并。观察器从不释放队列或改变业务/Run 状态。

## Checks 与 statuses

先完整分页列出 check suites，再逐套件 `filter=all` 分页 check runs，避免 ref 接口只覆盖最多
1000 个 suite 的限制。statuses 独立分页，不消费空 statuses 导致的 combined pending。
每个分页持续到空页；超过 1000 页拒绝整个快照。过滤后的 workflow runs 达到 GitHub 的
1000 结果上限时也拒绝快照，不把截断结果当作完整事实。Actions 读取 run 和当前 attempt 的完整 jobs，
绑定 workflow、event、head branch、suite、run、attempt 与 check_run_url；同一个 run number
冲突拒绝选择。历史 attempt 不可覆盖当前 job。普通 App 的多条同名 check 缺少可证明的替代
关系时保持 ambiguous；status 按受信 creator 与 context 选择最大 GitHub status ID。

## 实际验证边界

见 [GH-16 记录](quality/gh16/README.md)。HTTP fixture 的私仓权限测试是合成场景。
真实 `musutrade/disposable` 回查使用宿主 GitHub API，仓库是公开的；它只证明当时的外部事实，
不证明产品 Rust AppClient 的真实私钥路径、真实私仓或写能力已通过。首次部署仍需操作者在
控制面执行上述产品入口；普通预检不替代已授权 disposable 写路径实验。

接口依据：[installation token](https://docs.github.com/en/rest/apps/apps#create-an-installation-access-token-for-an-app)、
[Check runs](https://docs.github.com/en/rest/checks/runs)、
[workflow runs](https://docs.github.com/en/rest/actions/workflow-runs)、
[commit statuses](https://docs.github.com/en/rest/commits/statuses)。

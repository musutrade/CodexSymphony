# S2 spike — GitHub App 交接路径：installation token → 条件 push → PR → Checks → 合并

> 历史实验记录：结论仅适用于文中版本；“对方案的修正”和旧章节编号指 V9。当前实施以[综合方案 V10](../../Personal_AI_Software_Factory_综合方案.md)为准，旧建议不构成当前交付要求。原始实验与未验证项保留用于追溯。

当前权威规则：[11.1 仓库交付能力与 CI 契约](../../Personal_AI_Software_Factory_综合方案.md#111-仓库交付能力与-ci-契约)、[11.2 合并判定与证据采信](../../Personal_AI_Software_Factory_综合方案.md#112-合并判定与证据采信)。

日期：2026-09-08　　App `my-disposable-bot`（org `musutrade`）　　仓库 `musutrade/disposable`（public）
结论已写回综合方案 23.S、11.1、14.4、17.5。原始输出见 `findings_s2.json`（token 已脱敏）。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | App 权限 | 安装时只给了 `contents: write`、`pull_requests: write`、`metadata: read`，在本次 **public 仓库**完成 push、找/建 PR、读 Checks、合并。未额外授予 `checks: read` / `actions: read` 时，读取 check-runs / workflow runs 均 200；**不能据此推导 private 仓库权限已满足**。`branches/:b/protection` 403，`rules/branches/:b`（rulesets）200，`pulls/:n` 的 `mergeable_state` 可读；这些只记录本仓观察，不证明所有仓库保护条件都可由该字段替代。 |
| 2 | installation token | App JWT（RS256，`iss=app_id`，10 分钟）→ `POST /app/installations/:id/access_tokens` → 201，**有效期 1 小时**。可以按 `repositories` 限定到单仓库，可以按 `permissions` 向下收窄（contents:read 成功）；申请安装未持有的权限（checks）→ 422 "permissions requested are not granted"。 |
| 3 | 条件 push | 用 `git push --force-with-lease=refs/heads/<b>:<expected_sha> origin <sha>:refs/heads/<b>`，凭证走 `-c http.<url>.extraheader=AUTHORIZATION: basic base64(x-access-token:<token>)`，不进 URL、不落盘。远端与预期不符时 rc=1，stderr 精确为 `! [rejected] <sha> -> <branch> (stale info)`。新建分支用 expected=`0{40}`。REST 替代路径：`PATCH /git/refs/heads/<b>` 带 `force:false`，非 fast-forward → 422 "Update is not a fast forward"。 |
| 4 | push 幂等 | 远端已是目标 SHA 时重复 push rc=0、无输出。**注意**：此时即便 lease 的 expected 是错的（仍写 `0{40}`）也 rc=0——git 在"无需更新"时不检查 lease。对平台是正确语义（14.4 第 2 步"远端已是目标 SHA 则幂等成功"），但意味着 lease 不能当作"远端状态断言"，只能当作"更新守卫"。 |
| 5 | PR 查找 / 重复创建 | `GET /pulls?state=all&head=<owner>:<branch>&base=<default>` 精确命中，创建前 `[]`、创建后 1 条、合并后仍 1 条（state=closed, merged_at 非空）。重复 `POST /pulls` → 422，`errors[0].message = "A pull request already exists for <owner>:<branch>."`。PR 作者显示 `my-disposable-bot[bot]`。 |
| 6 | Checks 读取 | `commits/:sha/check-runs`、`check-suites`、`commits/:sha/status`、`actions/runs?head_sha=` 均 200。本轮 disposable 仓库无 workflow，全部 count=0、combined status 为 `pending`，不能将缺失的必需检查当作通过。combined status 汇总 commit statuses，不能替代 Actions check-runs；外部 status-only CI 仍按仓库契约验证所需 context，不能一概忽略。 |
| 7 | 条件请求 / 限流 | 返回 `ETag`；`If-None-Match` → 304 且 `X-RateLimit-Remaining` 不减。installation token 的限额 15000/h（resource=core）。`GET /rate_limit` 对 installation token 返回 404，限额只能从响应头读。 |
| 8 | 合并的 sha 守卫 | `PUT /pulls/:n/merge` 带 `sha` 参数：不符 → **409** `"Head branch was modified. Review and try the merge again."`；相符 → 200 `{sha, merged:true}`；本轮已合并后重复调用返回 200 和同一 SHA。`findings_s2.json` 还保存了 `GET /pulls/:n/merge` 返回 204，以及 PR 详情的 `merged:true`；这些是本次合并事实证据。未知响应或权限失败不表示未合并，不能据此盲目重发合并。 |
| 9 | Webhook | `installation.events = []`，App 未订阅任何事件；Phase 0 只轮询，符合方案。 |

## 对方案的修正

- **11.1 权限集（纠正旧推论）**：上述权限只在本次 public 仓库得到验证，不能据此删除 private 仓库 Checks、Actions 或 commit statuses 所需权限。接入仓库时按实际交付操作验证安装权限；读取 rulesets 成功也不等于已验证所有分支保护形态。
- **14.4 第 2 步**：实现方式定为 `git push --force-with-lease=<ref>:<expected>`，凭证经 `http.extraheader`；错误分类：stderr 含 `(stale info)` → `git_push_rejected`（对账，不重试）；其他 → `git_push_failed`（可重试）。远端已是目标 SHA → 成功，但要**另外 fetch 一次确认**，不能靠 lease 断言远端状态。
- **14.4 第 3 步**：find-or-create 的 find 查询定为 `state=all&head=<owner>:<branch>&base=<default>`；创建冲突 422 + 该 message 视为"已存在"，回到 find。
- **16.5 / 17.5 合并（纠正旧推论）**：本轮错误 head SHA 返回 409，成功 merge 响应 `{merged:true, sha}` 可以作为该动作结果；响应丢失时查询精确 PR 的可靠 `merged` / `merged_at` 事实，或以 `GET /pulls/:n/merge` 的 204 确认。404 只有在目标身份及访问权限已确认时才可解释为未合并；认证、授权、传输或解析失败仍是未知。`merge_commit_sha` 在合并前可以是 test merge SHA，非空不证明已经合并。[GitHub PR 字段语义](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)、[合并事实查询](https://docs.github.com/en/rest/pulls/pulls#check-if-a-pull-request-has-been-merged)
- **17.5**：`mergeable_state == "clean"` 是本轮观察，不单独证明全部仓库交付条件。异步 `mergeable == null` 需等待核对，`blocked` 不自动归类为代码失败。
- **11.5 / 19.3**：限流只能从响应头取；`GET /rate_limit` 对 installation token 不可用。
- **28 错误码**：新增 `github_merge_head_moved`（409），归 blocked，动作 = 放弃本次 MergeOperation 重等条件；`pr_create_failed` 中 422 "already exists" 不是失败，是 find 路径。
- **token 生命周期**：installation token 1 小时，Handoff Worker 每次操作前检查剩余有效期 < 5 分钟则重新换取；不缓存跨 Run 的 token。

以上旧章节编号保留历史追溯。关于同名 check-runs、重跑 attempt 与 combined status 的推论限制，见 [S2b 修正记录](README_S2b.md#对方案的修正)。[GitHub commit statuses 文档](https://docs.github.com/en/rest/commits/statuses#get-the-combined-status-for-a-specific-reference)说明 combined status 的汇总对象，不能将本次无 status 的 `pending` 泛化为所有 CI 的结论。

## 复现

```bash
export FACTORY_GH_APP_ID=<app id>
export FACTORY_GH_APP_KEY_PATH=~/.secrets/<app>.private-key.pem   # chmod 600
export FACTORY_GH_REPO=musutrade/disposable                       # 默认值
python3 spikes/s2/s2_github_app_handoff.py
```

脚本会在目标仓库上：建分支 `ai/req-s2-<rand>` → 条件 push → 用 REST 制造一次外部提交 → 建 PR → 读 Checks → 用 sha 守卫合并到默认分支 → 删分支。**会真实合并一个 squash commit 到默认分支**，只在 disposable 仓库上跑。私钥不落入仓库；请在 spike 结束后 rotate。

## 未验证

- private 仓库下读 check-runs / actions runs 是否需要 `checks: read` / `actions: read`。
- 有真实 workflow 时 check-run 的 `name` 格式（决定 17.5 "必需 Check 名称集合"的配置形态）——需要给 disposable 加一个 Actions workflow 再测。
- 有 branch protection / ruleset 的仓库下 `mergeable_state` 的取值序列（`blocked` → `clean`）。
- installation token 过期瞬间的错误形态（401 vs 403）。
- 组织级 App 安装到多个仓库时的 token 按仓库限定（S2 只有一个仓库）。

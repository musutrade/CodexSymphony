# S2 spike — GitHub App 交接路径：installation token → 条件 push → PR → Checks → 合并

日期：2026-09-08　　App `my-disposable-bot`（org `musutrade`）　　仓库 `musutrade/disposable`（public）
结论已写回综合方案 23.S、11.1、14.4、17.5。原始输出见 `findings_s2.json`（token 已脱敏）。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | App 权限 | 安装时只给了 `contents: write`、`pull_requests: write`、`metadata: read`，**足以完成 push、找/建 PR、读 Checks、合并全流程**（public 仓库）。`checks: read` / `actions: read` 在 public 仓库下读取 check-runs / workflow runs 均 200；**private 仓库是否需要显式 `checks: read` 未验证**。`branches/:b/protection` 403（需 `administration: read`），但 `rules/branches/:b`（rulesets）200 且 `pulls/:n` 的 `mergeable_state` 可用，17.5 不需要 administration 权限。 |
| 2 | installation token | App JWT（RS256，`iss=app_id`，10 分钟）→ `POST /app/installations/:id/access_tokens` → 201，**有效期 1 小时**。可以按 `repositories` 限定到单仓库，可以按 `permissions` 向下收窄（contents:read 成功）；申请安装未持有的权限（checks）→ 422 "permissions requested are not granted"。 |
| 3 | 条件 push | 用 `git push --force-with-lease=refs/heads/<b>:<expected_sha> origin <sha>:refs/heads/<b>`，凭证走 `-c http.<url>.extraheader=AUTHORIZATION: basic base64(x-access-token:<token>)`，不进 URL、不落盘。远端与预期不符时 rc=1，stderr 精确为 `! [rejected] <sha> -> <branch> (stale info)`。新建分支用 expected=`0{40}`。REST 替代路径：`PATCH /git/refs/heads/<b>` 带 `force:false`，非 fast-forward → 422 "Update is not a fast forward"。 |
| 4 | push 幂等 | 远端已是目标 SHA 时重复 push rc=0、无输出。**注意**：此时即便 lease 的 expected 是错的（仍写 `0{40}`）也 rc=0——git 在"无需更新"时不检查 lease。对平台是正确语义（14.4 第 2 步"远端已是目标 SHA 则幂等成功"），但意味着 lease 不能当作"远端状态断言"，只能当作"更新守卫"。 |
| 5 | PR 查找 / 重复创建 | `GET /pulls?state=all&head=<owner>:<branch>&base=<default>` 精确命中，创建前 `[]`、创建后 1 条、合并后仍 1 条（state=closed, merged_at 非空）。重复 `POST /pulls` → 422，`errors[0].message = "A pull request already exists for <owner>:<branch>."`。PR 作者显示 `my-disposable-bot[bot]`。 |
| 6 | Checks 读取 | `commits/:sha/check-runs`、`check-suites`、`commits/:sha/status`、`actions/runs?head_sha=` 均 200。disposable 仓库无 workflow，全部 count=0、combined status `pending`——**验证了 17.5 "缺失不算通过"的必要性**：没有 CI 的仓库 combined state 永远是 pending，不是 success。 |
| 7 | 条件请求 / 限流 | 返回 `ETag`；`If-None-Match` → 304 且 `X-RateLimit-Remaining` 不减。installation token 的限额 15000/h（resource=core）。`GET /rate_limit` 对 installation token 返回 404，限额只能从响应头读。 |
| 8 | 合并的 sha 守卫 | `PUT /pulls/:n/merge` 带 `sha` 参数：不符 → **409** `"Head branch was modified. Review and try the merge again."`；相符 → 200 `{sha, merged:true}`；**已合并后重复调用 → 200 且返回同一 merge sha**（幂等，不是 405）。`GET /pulls/:n/merge` 204=已合并 / 404=未合并，可作为响应丢失后的事实查询。合并者显示为 App bot。 |
| 9 | Webhook | `installation.events = []`，App 未订阅任何事件；Phase 0 只轮询，符合方案。 |

## 对方案的修正

- **11.1 权限集**：Phase 0/1 最小集确认为 `contents: write` + `pull_requests: write` + `metadata: read`。`checks: read` 和 `actions: read` 从"必须"降为"private 仓库建议加，待验证"。删掉 `commit_statuses`（`commits/:sha/status` 用 contents 权限即可读）。**不需要** `administration`——分支保护改用 rulesets API 或 `mergeable_state`。
- **14.4 第 2 步**：实现方式定为 `git push --force-with-lease=<ref>:<expected>`，凭证经 `http.extraheader`；错误分类：stderr 含 `(stale info)` → `git_push_rejected`（对账，不重试）；其他 → `git_push_failed`（可重试）。远端已是目标 SHA → 成功，但要**另外 fetch 一次确认**，不能靠 lease 断言远端状态。
- **14.4 第 3 步**：find-or-create 的 find 查询定为 `state=all&head=<owner>:<branch>&base=<default>`；创建冲突 422 + 该 message 视为"已存在"，回到 find。
- **16.5 / 17.5 合并**：`sha` 守卫的失败码是 409，不是 405/422；已合并后重复 merge 是 200 幂等，`merged` 事实用 `GET /pulls/:n/merge` 的 204/404 判定，不用 merge 接口的返回值。
- **17.5**：`mergeable_state == "clean"` 可作为 branch protection 已满足的信号，无需 administration 权限；但它是 GitHub 异步计算的（首次 GET 可能为 null），Reconciler 要处理 null → 重试。
- **11.5 / 19.3**：限流只能从响应头取；`GET /rate_limit` 对 installation token 不可用。
- **28 错误码**：新增 `github_merge_head_moved`（409），归 blocked，动作 = 放弃本次 MergeOperation 重等条件；`pr_create_failed` 中 422 "already exists" 不是失败，是 find 路径。
- **token 生命周期**：installation token 1 小时，Handoff Worker 每次操作前检查剩余有效期 < 5 分钟则重新换取；不缓存跨 Run 的 token。

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

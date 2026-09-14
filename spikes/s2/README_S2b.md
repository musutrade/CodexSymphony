# S2 补测 — 真实 workflow 的 check-run 名称与 mergeable_state 序列

> 历史实验记录：结论仅适用于文中版本；“对方案的修正”和旧章节编号指 V9。当前实施以[综合方案 V10](../../Personal_AI_Software_Factory_综合方案.md)为准，旧建议不构成当前交付要求。原始实验与未验证项保留用于追溯。

当前权威规则：[11.1 仓库交付能力与 CI 契约](../../Personal_AI_Software_Factory_综合方案.md#111-仓库交付能力与-ci-契约)、[11.2 合并判定与证据采信](../../Personal_AI_Software_Factory_综合方案.md#112-合并判定与证据采信)。

日期：2026-09-10　　仓库 `musutrade/disposable`（含 `.github/workflows/ci.yml`：job 名 `test-job`，
ruleset 要求该 check 通过才能合入 `main`）
补充 `spikes/s2/README.md`；结论已写回综合方案 23.S、11.5、17.5。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | check-run 名称形态 | 本轮 `check_runs[].name == "test-job"`，等于该 workflow 的 job `name`，发布 App 为 `github-actions`。这不证明所有 workflow 的命名模式，也不意味着仅按显示名称即可区分检查来源。 |
| 2 | combined status | 本轮仓库只有 Actions check-runs，`commits/:sha/status` 的 `total_count == 0`、`state == "pending"`，观察窗口内直到合并均未变化。combined status 不能替代 Actions check-runs；外部 status-only CI 仍按仓库契约核对必需 context，不因本次观察而被排除。 |
| 3 | `mergeable_state` 序列 | `null` + `mergeable: null`（首次读取，GitHub 还在算）→ `blocked`（check 已 success，但 ruleset 尚未满足/仍在计算）→ `clean`。时间点：t=12s 未知、t=21s blocked、t=30s clean。→ Reconciler 必须处理 `mergeable == null`（重试），且 `blocked` 不一定代表失败。 |
| 4 | 同一 SHA 的 check-runs | 本轮 push 与 pull_request 触发后，返回两条同名 `test-job`，最终均为 success，App 均为 `github-actions`。原始 findings 未保存它们的 workflow/run/attempt 身份，不能据此得出“同名所有历史 run 都必须成功”的通用规则；本轮没有测失败后重跑。 |
| 5 | 早于 CI 的合并 | 未单独构造“CI 未完成即合并”的用例。`blocked` 观察与 head SHA 守卫反例不能替代该用例，也不能据此宣称任意安装权限或仓库规则下都会拒绝提前合并。 |
| 6 | sha 守卫（本轮复现） | `PUT /pulls/:n/merge` 带错误的 `sha`（`base_sha`）→ **409** "Head branch was modified."；带正确 `sha` → 200 + merge sha。与 S2 首轮一致。 |
| 7 | 合并结果与证据限制 | `findings_s2b.json` 对本轮合并只保存 `merge_result.status=200` 和 SHA `0b0dd251d1a3e9b1eead6dd1e13a04e246c82613`。旧文“`GET /pulls/3` 返回 `merged:null`”没有原始响应证据，撤回该推论及对其他 REST 视图的泛化；不能把这个字段观察写成已验证事实。 |

## 对方案的修正

- **17.5（限定观察）**：`mergeable_state == "clean"` 不单独证明全部交付条件；先处理 `mergeable == null`，`blocked` 表示条件尚待核对，不直接判为代码失败。
- **11.5（纠正同名聚合）**：按仓库契约匹配精确源码、检查来源、workflow/job、阶段与当前有效 attempt。旧失败 attempt 已被有效重跑取代时不应永久卡住；也不能从同名结果中随意挑一条绿灯。GitHub check-runs API 默认 `filter=latest`，这不自动证明跨 workflow、套件和事件的必需检查已正确归组；必要时读取完整分页和关联身份。此处是修正后的设计要求，不是本 spike 已完成的重跑验证。[GitHub check-runs 文档](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference)
- **11.5（纠正 status 泛化）**：combined status 汇总 commit statuses，不能替代 check-runs；仓库若使用外部 status-only CI，仍按契约核对该来源的必需 context。缺少预期 status/check 不能由其他来源成功代替。[GitHub commit statuses 文档](https://docs.github.com/en/rest/commits/statuses#get-the-combined-status-for-a-specific-reference)
- **17.5（限定名称）**：`test-job` 只是本次 job 名观察；名称和 App 显示值不是完整的 workflow/attempt 身份。
- **14.4 / 22.5（纠正合并事实）**：以精确 PR 的可靠 `merged` / `merged_at` 事实或 `GET /pulls/:n/merge` 的 204 确认已合并。`merge_commit_sha` 在合并前可能指向 test merge commit，**非空不代表已合并**；字段缺失/未知、权限失败也不能当作未合并。404 需先确认目标身份与访问权限，不能直接据此触发新的合并动作。[GitHub PR 字段语义](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)、[合并事实查询](https://docs.github.com/en/rest/pulls/pulls#check-if-a-pull-request-has-been-merged)

2026-09-14 复核仅修正文档推论，未重跑 spike 或改写脚本、findings。S2 首轮的 `findings_s2.json` 确实保存了 `merged:true` 和合并查询 204；不能把 S2b 缺失的原始 GET 响应补写成与其相反的实验结果。本实验仍只覆盖 public 仓库，private 权限边界见 [S2 未验证项](README.md#未验证)。

## 复现

```bash
export FACTORY_GH_APP_ID=<id>
export FACTORY_GH_APP_KEY_PATH=~/.secrets/<app>.private-key.pem
python3 spikes/s2/s2b_checks_and_mergeable.py
```

脚本会建分支/PR、轮询 check-runs 与 `mergeable_state`、验证 sha 守卫、在 clean 时合并、清理分支。
输出见 `findings_s2b.json`。**会在 disposable 仓库的 main 上留下一个 squash commit**。

## 未验证

- CI 失败时的 `mergeable_state`（预期仍为 `blocked`）与 check-run 的 `conclusion` 形态（`failure`）。
- CI 未完成（`status: in_progress`）时 `mergeable_state` 的取值。
- 多个必需 check 时 `blocked` → `clean` 的完整梯度。
- PR 修改后 head 变化对 `mergeable_state` 的回退行为（本轮用例 head 未变）。
- GitHub 计算 `mergeable` 的耗时分布（本轮 12s 内为 null，属正常范围）。
- 同一 SHA 首次失败后重跑成功、跨 workflow 同名 job、部分重跑与多套件分页的有效 attempt 归属。
- status-only 或混合 CI 的契约匹配；private 仓库读取及恢复 CI 所需权限。
- S2b 合并后 PR 详情完整响应，以及合并查询失败/权限不足的分类；本轮 findings 不包含这些结果。

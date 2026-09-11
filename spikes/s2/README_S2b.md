# S2 补测 — 真实 workflow 的 check-run 名称与 mergeable_state 序列

日期：2026-09-10　　仓库 `musutrade/disposable`（含 `.github/workflows/ci.yml`：job 名 `test-job`，
ruleset 要求该 check 通过才能合入 `main`）
补充 `spikes/s2/README.md`；结论已写回综合方案 23.S、11.5、17.5。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | check-run 名称形态 | `check_runs[].name == "test-job"`——**等于 workflow 里 job 的 `name`**，不是 `CI / test-job` 这类复合名，也不是 workflow `name`（`CI`）。发布 App 为 `github-actions`（`app.slug == "github-actions"`）。→ 17.5 的"必需 Check 名称集合"直接配 job 名即可。 |
| 2 | combined status | **永远是 `pending`**：仓库只有 Actions check-runs 时 `commits/:sha/status` 的 `total_count == 0`、`state == "pending"`，直到合并都没变。→ 绝不能把 combined status 当作 CI 判据；必须用 check-runs。（S2 首轮在无 workflow 的仓库见过同样现象，这里确认它在有 workflow 时依旧。） |
| 3 | `mergeable_state` 序列 | `null` + `mergeable: null`（首次读取，GitHub 还在算）→ `blocked`（check 已 success，但 ruleset 尚未满足/仍在计算）→ `clean`。时间点：t=12s 未知、t=21s blocked、t=30s clean。→ Reconciler 必须处理 `mergeable == null`（重试），且 `blocked` 不一定代表失败。 |
| 4 | 同一 SHA 的 check-runs | 返回 **2 条同名的 `test-job`**（push 与 pull_request 两个事件各触发一次），`app.slug` 都是 `github-actions`。→ 判据必须是"该名称下**全部** run 都 success"，不能只取第一条；同名多 run 是常态。 |
| 5 | 早于 CI 的合并 | 未单独构造"CI 未完成即合并"的用例；但 ruleset 生效时 `mergeable_state` 为 `blocked`，配合 sha 守卫（第 6 条）足以在服务端拒绝。 |
| 6 | sha 守卫（本轮复现） | `PUT /pulls/:n/merge` 带错误的 `sha`（`base_sha`）→ **409** "Head branch was modified."；带正确 `sha` → 200 + merge sha。与 S2 首轮一致。 |
| 7 | 合并后状态 | PR #3 合并后 `main` head 变为 squash commit（`0b0dd25`，message `S2b c2e87b5d (#3)`）；`GET /pulls/3` 返回 `state: closed` 且 `merged` 字段为 `null`——**合并事实要看 `merge_commit_sha` / `merged_at` 或 `HEAD` 是否包含该 PR，不能只看 `merged` 布尔**（REST 在 list 视图里该字段常为 null）。 |

## 对方案的修正

- **17.5**：`mergeable_state == "clean"` 作为合并条件成立，但必须配套两条：
  ① 先处理 `mergeable == null`（重试几秒）；② `blocked` 视为"条件尚未满足"而非失败。
- **11.5**：CI 判据固定为 check-runs（按 `name` + `app.slug` 匹配，取该名称下全部 run），
  **不用 combined status**；combined status 只作展示，且要标注"Actions 仓库下恒为 pending"。
- **17.5**：必需 Check 名称集合按 job `name` 配置（本例 `test-job`），不需要 `CI /` 前缀。
- **14.4 / 22.5**：合并事实判定加一条——`GET /pulls/:n` 的 `merged` 可能为 `null`，
  以 `merge_commit_sha` 非空或 `GET /pulls/:n/merge` 的 204 为准。

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

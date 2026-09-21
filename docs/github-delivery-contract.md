# GH-84：版本化交付能力与 CI 证据

GH-84 增加显式 V1 能力契约和只读证据判定，不执行 rerun、合并或业务 Done。
继续使用 `github::Policy`、`github_observe`、`github_store` 和控制面 AppClient；
Requirement、AgentRun 和 GitHub 事实不互相改写。

## 启用与旧策略迁移

没有 `policy.delivery`（或值为 null）的配置仍按原 M1/M2 交接范围运行，不自动获取 V1 权限。
启用 V1 时先通过现有仓库登记更新策略版本、评审受影响需求，再在受信控制面配置
同一仓库 ID、remote、默认分支和新 `version`。已有策略不能在同一版本下增加或
改写 V1 delivery；`configure` 返回 false，不覆盖原记录。新版本能力核验通过前不能领取。
旧 JSON 缺失可选字段可直接读取；没有批量扩大业务授权的迁移。

`delivery.schema_version=1`。必填结构由 `github_contract.rs` 定义：

| 字段 | 要求 |
|---|---|
| `pre_merge.source` | `head` 或 `test_merge`，指定 Checks/status 关联 SHA |
| `pre_merge.checkout` | `head` 或 `test_merge`，独立指定预期实际检出 SHA |
| `pre_merge.checks` | 与旧 `policy.required` 完全一致的 selector 投影，加适用性、触发和 job |
| `pre_merge.wait_seconds` | 从阶段持久化开始时间计算的等待上限，不随轮询续期 |
| `post_merge` | 独立配置 `checks` 或 `fixed_validation`，只能绑定已确认的最终 merged SHA |
| `actions` | 显式设置 `rerun_actions`、`rerequest_checks`、`merge`、`merge_method`、`read_logs` |
| `protection` / `rules` | 经审阅的完整分支保护响应及有效 rules 列表；实时结果变化即阻塞 |

每个 `checks` 项包含：

```json
{
  "selector": {
    "name": "build",
    "source": {
      "kind": "actions", "app_id": 15368,
      "workflow_id": 123, "workflow_sha": "REVIEWED_GIT_BLOB_SHA",
      "event": "pull_request", "branch": "reviewed-branch",
      "branch_from_pr": false
    }
  },
  "applicability": "always",
  "trigger": {"kind": "pull_request"},
  "job": "build"
}
```

示例 ID/SHA 不是可用授权。Actions 必须绑定 App、workflow、固定配置 blob、event、
branch、job 名和 check-run URL，并保存 run/suite/attempt。`branch_from_pr=true`
仅沿用显式同仓 PR 分支授权，post_merge 不允许使用它。当前只支持 always 适用项；
不能把 conditional skipped、neutral 或 N/A 当作成功。

外部 check-run/status 的 trigger 使用
`{"kind":"external","path":"受审触发配置文件","blob_sha":"固定blob","event":"pull_request或push"}`；
默认分支和每次观察选定的 pre/post SHA 上的配置均须匹配。外部触发器的语义由该受审配置授权，
不是从检查名称推断。预检还要求已有精确 SHA 的可信来源可解析。

post_merge 的 Checks 路径：

```json
{
  "kind": "checks", "wait_seconds": 600, "probe_pr": 42,
  "checks": [{
    "selector": {"name": "build", "source": {
      "kind": "actions", "app_id": 15368,
      "workflow_id": 456, "workflow_sha": "REVIEWED_POST_MERGE_BLOB_SHA",
      "event": "push", "branch": "main", "branch_from_pr": false
    }},
    "applicability": "always", "trigger": {"kind": "push"}, "job": "build"
  }]
}
```

`probe_pr` 必须是同仓已有且可靠确认合并的只读探针，其实际 merged SHA 必须提供
可解析的对应 post-merge 来源。其触发只能是 push/workflow_dispatch；PR-only 不能借用
pre_merge 绿灯。workflow_dispatch 需要 Actions write，预检不会发起 dispatch。
历史探针证明能力，不代替当前业务 PR 的证据；缺失/不适用触发在领取前阻塞。

固定验证路径：

```json
{"kind":"fixed_validation","plan_id":"/absolute/control-plane/approved-plan.json","configuration_sha256":"64位受信计划摘要","authorization":"原授权引用","wait_seconds":600}
```

`plan_id` 引用已有 `validation_runner::Plan` 文件。预检读取计划，检查绝对入口路径、
受保护入口内容摘要、允许的步骤/期限和 `Plan::identity().config_sha256`；缺失或变化阻塞。
只读预检不执行计划。后续阶段通过已有独立验证器取得证据，
`PhaseEvidence::bind_validation` 校验 candidate、trusted identity、步骤结果和实际源码身份。
GitHub Actions 的 `head_sha` 不是 checkout 证明；未知时 `actual_checkout_sha=null`，
即使 CI 绿色也不声称阶段证据完整。fixed_validation 无须创建空提交或 PR。

## 权限、保护与失效

沿用按仓库限域的 Contents / Pull requests write、Checks / Actions read；V1 增加
Administration read 读取完整保护。选择 statuses 时请求 Commit statuses read。
只有配置 Actions rerun/dispatch 才请求 Actions write；只有 rerequest_checks 才请求 Checks write。
V1 每次能力核验重新取得授权范围，策略变化也使 token 缓存失效。

`read_logs=true` 核对 pre/post 已选 job 的日志下载，要求带 Location 的 302，
再无凭证下载已知 GitHub 日志存储域的内容（30 秒、8 MiB 上限）。只保存 job ID、字节数及摘要，
不保存短期签名 URL 或原文，不把 App token 转发给下载存储。空日志、下载失败、
未支持的存储域或外部发布者缺少日志适配器时明确阻塞。
所选 merge 方法还必须在仓库启用；需要平台无法满足的独立 review 时阻塞，
不自行批准或降低仓库保护。test-merge 要求 strict 检查、管理员强制及必需检查，
避免依赖旧 base 的证据被用于未经验证的新 base。

能力维持 30 秒刷新、60 秒有效期，领取事务再次核对仓库/评审版本、策略、stale 和缺项。
V1 拒绝未来观察时间。权限、工作流 blob、保护或默认分支不匹配，须修正配置并重新核验。
观察前后 PR 内容不一致时整个快照无效。消费阶段证据须用
`Observation::evidence_current` 核对当前 policy/head/base/时间，不能仅检查绿色结果。

## 检查与合并事实

- Checks suites、runs、statuses、Actions runs/jobs 分别完整分页；截断或溢出失败关闭。
  不消费空 statuses 导致的 combined pending；status-only 单独选择可信 creator/context。
- 同 SHA、同 workflow/run 身份下当前有效 attempt 的 job 可以替代旧失败；同 run number
  的冲突身份、重复映射、未知字段不能挑绿灯。不同发布者不能借用同名成功。
- `Check.evidence` 保存选中身份，`Check.history` 保存旧来源原件；旧失败不改写为重跑成功。
  generic App check 无可证明替代关系时仍是 Ambiguous。
- `merged=true` 或可靠非空 `merged_at` 才确认 merged；closed 或非空 merge_commit_sha
  均不能单独证明已合并。不可访问的 403/404 是未知读取错误，不改写成 Unmerged。
- `PhaseEvidence::state(started_at, now)` 将没有完成的证据在固定 deadline 后判为 Failure；
  start 由后续持久化阶段控制器提供，不能传每次轮询的开始时间来延长等待。

## API 与界面

`GET /api/multi/repository` 每个仓库新增可选字段 `capability_blockers`、
`capability_checked_at`（Unix 秒或 null）、`capability_stale`、`capability_error` 和
`capability_http_status`（错误代码/HTTP 状态，无错误时 null）。`delivery_ready` 仍是当前
可领取能力布尔值。仓库工作台显示具体缺口和过期提示，管理员修正所列来源、权限、
触发或保护配置后重新核验；用户不需要删必需检查来让提示消失。
原 `/api/repository` 通过既有 legacy projection 保持关闭 schema，不新增这些字段。
PR 详情继续从已有 operations GitHub observation 展示 `phases`、source/history 和 SHA 身份。

## 持久化与回退

迁移 `0025_github_evidence.sql` 为现有 GitHub 事实增加 `github_evidence_history`，
回填旧快照并使能力/PR stale，等待重新读取；不修改业务或 Run 状态。
每次保存（包括迟到结果）先归档原件，再以策略和观察时间 CAS 更新当前快照。
更旧结果不能覆盖新事实；同一时间的冲突使快照 stale。旧策略的观察不能覆盖新策略。
历史表不是独立真相来源，也不独立推动队列。

回退需先暂停队列、停止写入并保全数据库。保留 0025 及历史数据，不删除失败原件；
优先回退应用到兼容新增可选字段和迁移记录的修复版本。旧二进制若不认识 migration 0025
不得直接作为数据库降级工具。恢复 M1/M2 范围须新授权版本及重新评审，不能删除 delivery
字段来追溯扩大或变更已领取需求的权限。

## 验收边界

可复跑：fixture wrapper 下运行 `cargo test --workspace --locked`、
`cargo clippy --workspace --all-targets --locked -- -D warnings` 和 `cargo fmt --all -- --check`；
`tests/github_support/v1.rs` 与原 `tests/github.rs` 共同覆盖 V1/旧策略、公开/私有响应、
check-only/status-only、PR-only、分页、伪来源、重跑、乱序、未知合并及 head/base 身份变化。
固定验证计划使用临时合成资源；HTTP fixture 不冒充真实私仓 App 或外部写路径验收。

前端执行 checked-in npm lint/test/build；M2 认证后的桌面/手机验收使用已有
`python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py` 隔离 HTTPS fixture。
通用 `/opt/symphony-env/e2e.py` 仍传入 HTTP origin，会被当前产品拒绝，不能据此降低 HTTPS 要求。
所有会启动产品 API 的套件必须串行，保留产品单实例锁。完整 Rust 套件使用干净的
供给 test 数据库；恢复测试会自行创建新数据库，不在 TEST_DATABASE_URL 追加专用 search_path。
本地源码风险采集不签发 Gate。远端精确 PR head 的 Harness-Gate 与 Trusted Harness-Gate
仍由独立宿主执行；本项不运行生产部署、通知或真实 GitHub rerun/merge。

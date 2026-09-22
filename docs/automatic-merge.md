# M3-3 自动合并与子项验收

本项为 GH-86；以已合入的 #84（PR #93）和 #85（PR #94）为前置。
只有显式启用 `delivery.actions.merge`、仓库版本与授权快照一致的任务进入本流程。
旧 0a 观察保留；升级程序或登记新策略不会追溯扩大已有任务授权。

## 身份与推进

1. 从已成功的 `candidate_validation` 和平台 `delivery` 读取候选，核对当前
   revision、仓库版本、整组授权、累计预算、依赖、全局 owner、恢复屏障、
   暂停/取消、在途 Run/修复/工作保全/交接。读取实际 PR 的全部必需检查。
2. `merge_operation` 先保存精确 repo/PR/head/base、实际 pre_merge checkout、
   策略、validation ID、授权及依赖快照；其摘要为 action_key。合并前重新读取
   仓库能力及 PR，要求 head/base 不变、mergeable/clean、非 draft、强制 strict
   checks 和管理员保护。不绕过 review 或保护，也不支持不具备该保障的自动合并。
3. Broker 在事务提交 `unknown` 和 `merge_started` 后，只发送一次带 expected
   head SHA 的合并请求。超时、429、404、权限丢失、重启均先读取同一 PR；
   未知期间不重新编码、重建 PR、重复 Merge 或释放 owner。
4. 确认合并后记录实际 merged SHA。成功写回执与该 SHA 一致时保存实际合并方式；
   丢失回执时不把配置的期望方式当作实际方式。后者仍读取、验证实际合并版本，
   但保留占用并显示方式未确认的阻塞，不猜测 squash/rebase 的 Git 拓扑。
5. 在独立 worktree 执行适用 post_merge 计划。PR-only 仓库使用策略中批准且
   摘要匹配的固定计划，无需等待不存在的 CI。需要 workflow_dispatch 时先保存
   精确 workflow/ref/SHA 意图；响应未知只观察原动作，不重复发送。
6. test-merge 验证与 merged checkout 验证分别保存源码、受信入口和原始输出。
   绝不改标 head/test-merge 证据。完整 merged checkout 验证通过、远端再次核对且
   当前授权仍适用后，真实受信适配器事务写入 `group_completion` 与 Requirement
   `Done`。AgentRun 和原 CI 失败事实不重写。
7. 同仓后继基线必须包含依赖合并提交；跨仓仍按既有授权版本/产物关系验证。
   依赖读取接受已完成的 `Done`，兼容原 `Submitted` 记录。validation_only 和父项
   集成完成不由本项实现。

合并后失败保留已合并 SHA、日志和 owner，并报告“原项恢复 / M3-5 unavailable”。
不创建后继修复项绕过占用，不回滚 main，不伪造 Done。暂停/取消或授权变化会停止
合并阶段的本地验证进程组；原始中断调用保留且不能无条件重跑。未知本地执行、
未知远端结果均不能靠重启清空。取消与已发送 Merge 竞态先对账，取消不产生完成事实。

## 迁移与回退

`0027_automatic_merge.sql` 是增量迁移：新增 merge_operation；为 candidate_validation
增加 approved_plan；Requirement state 约束增加 Done。已存在的验证记录不伪造计划：
缺失 approved_plan 时，需要该计划的自动路径安全阻塞。固定 post_merge 计划仍须
经过策略授权、入口摘要和所需 AC 步骤检查。无 delivery 契约的旧仓库不自动启用。

先在独立 disposable 数据库执行通常的服务迁移，再启用新的仓库策略版本并重新
授权适用任务。保留升级前备份、原失败输出、迁移回执和源码身份。

回退先暂停队列、停止新写、对账 unknown 操作并确认本地工作已静止；保存数据库、
merge_operation 和所有验证目录。通过新的明确仓库策略版本关闭自动合并。
不要在 unknown、merged 或 blocked owner 上删除表、删除证据、重置 stage，或把
Done 改成 Submitted。旧二进制没有这套 owner/验收语义；有未终结合并时不得直接
降级运行。需要降级时在隔离副本恢复升级前备份并先核对升级后的真实远端事实，
不能用 SQL 回滚伪造已经发生的 GitHub 合并。

## 可复跑本地验收

使用当前 Issue 的一次性 PostgreSQL 注入环境：

```sh
python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/gate.py config check
python3 tools/gate.py verify --profile hook --all
```

`tests/automatic_merge.rs` 覆盖精确身份、旧证据、base/head 变化、限流/404/权限丢失、
未知响应、冷启动、当前授权/恢复屏障/写占用复核。
`tests/github_support/merge.rs` 经真实 HTTP fixture、持久化 Broker 和真实本地 Git/验证
执行三种合并方式、test-merge 与 merged 身份分离、Done 后同仓后继领取、响应丢失、
暂停/取消竞态及合并后失败入口。HTTP fixture 的 GitHub 响应是合成的；这些测试
不是公网 GitHub 合并验收。
`tests/validation_runner.rs` 验证受信入口、源码完整性、输出保留、进程组停止及中断
调用禁止重放。完整现有 Runtime、交接和 0a 观察测试同样必须通过。

完整精确提交 Harness-Gate / Trusted Harness-Gate 由独立宿主执行；本地 hook 不替代
CRAP、覆盖率、完整 CI 或可信签名证据。

## 真实 disposable 验收（2026-09-22）

指定 runner 为 `python3 /opt/symphony-env/product-acceptance.py`，目标仅
`musutrade/disposable` / repository_id=1360824360，使用独立 tmpfs 数据库。
宿主安装回执绑定产品源码 `d6e5d5f053053f05d0b6490967bd3304c46aac2f`、
源码树 `132d5fc53ca4bfe1fc0f348d75e2282b32ce44eb`、二进制 SHA-256
`39e5556d06a11f0860e3fdfd4095afcf382bf7859d52b2943280de64188f2ee4`。
后续文档更新不改变该产品代码。保护策略 v8602 保持 strict/admin 与
`test-job` / App=15368；固定 post_merge 计划摘要为
`bb0d56177a19b7e7342d647e7121e9a9894c85413357206225a24d6da9a6fe46`。
宿主凭据未进入工作区，未改生产环境或扩大预算。

| 子项 | 真实 PR head | 实际 merged SHA / acceptance SHA |
|---|---|---|
| [PR #5](https://github.com/musutrade/disposable/pull/5) | `2cc377ac215c2ceb3421e35e0eab517029dcdb3f` | `59a66ba0a08d257e6bfb18a7cf85e6ec342dd2ef` |
| [PR #6](https://github.com/musutrade/disposable/pull/6) | `75d948afcf64759e8abfdcdb865351997d3dd704` | `6b6e2fb0e0f3f0bc2b56553b886290edfed6f36d` |

真实 Runtime 生成候选，产品负责提交、PR、Broker 合并和独立 merged checkout 验证。
两个 `merge_operation` 均为 complete，`group_completion` 来自
`platform-merged-checkout-validation/v1`，其 acceptance SHA 与实际 merged SHA 一致。
后继基线为第一项 merged SHA；两个 Requirement 均为 Done，owner 已释放。
父项仍为 waiting_business_acceptance，未宣称父项集成验收或全部 B01/B08 完成。
本次真实外部验收使用 squash；merge/rebase 和故障竞态由前述持久化 HTTP fixture
测试覆盖，不宣称这些场景都在公网实测。

原始第一次 Run 因忽略目录的 Git staging 失败而 Interrupted；恢复保留原 Run、
工作快照和累计用量。修复缓存路径 staging 与最后一个已预留模型回合后的非模型
合并准入后，沿原授权继续执行，没有注入完成 Fact。原失败与成功原件分别保留。

证据位于工作区 `artifacts/gh86/commit-recovery-20260922/`：
`final-acceptance-evidence.json` 保存动作回执、实际合并提交上的验证输出、依赖基线、
完成身份、预算和 owner；`first-merge-successor-evidence.json` 保留首次完成时的状态。
`artifacts/gh86/product-acceptance-runner/runner-receipt.json` 绑定宿主源码与二进制。
`artifacts/gh86/resume-status-current.json` 是本次只读复核的终态回执。
所有交付证据由工作区根 `.symphony-evidence.json` 按 SHA-256 声明，交给控制器归档。

复核已有验收使用 runner 的 `acceptance-status`，不要重复 `acceptance-start`。
重新执行需要新的明确 disposable 测试授权与独立数据：先验证源码/二进制、仓库保护、
策略与计划身份，再授权代码子项及依赖后继；让平台生成候选、等待真实 PR CI、执行
Broker 合并并保存 merged checkout 验证和后继基线。不能使用直接 GitHub Merge
代替产品 Broker，也不能复用已有完成记录作为新版本验收。

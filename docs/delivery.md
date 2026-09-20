# Phase 0a 可靠交接（GH-21）

有效候选完成验证时，同一个 PostgreSQL 事务写入 `candidate_validation` 的结果、
`delivery` 的不可变产物清单和 `delivery_action` 的发布意图。稳定 action_key 绑定
Requirement/revision、仓库数字身份/名称、目标分支、base 与 SHA。验证 ID、源 Run、
交付尝试 ordinal、精确 PR/head 与当前 consumer 保持关联；本项不执行证据归并。

单个后台 GitHub worker 在每次重放先查询同仓同分支的全部 PR，再读取分支 head。
发送前检查当前授权、唯一队列、暂停/取消、静止和保全、实际存储，以及有效候选。
App token 只由平台取得；普通 Git 仅 fast-forward push，不使用 force，不创建替代分支。
PR 必须同时匹配 repo/base/branch/SHA 和动作标记；关联、观察证据和 Submitted 同事务保存。
网络响应仅记录在具体 attempt，独立回读才确认。DB 提交失败不丢失原意图。

发布和关闭默认分别最多三个写尝试。每次写之前落库 unknown；失败按 30/120 秒退避，
耗尽或冲突进入同一动作的 blocked。阻塞/暂停期间继续有界只读对账，60 秒间隔，
不启动模型。取消后的未知创建不能被一次空列表证伪，保留占用等待真实 PR 或人工对账；
未知 push 必须读到精确 head 才能撤销发布意图。分支和提交始终保留。

暂停先保存意图并停止本地 Run；在途远端结果仍记录。恢复不清除取消或存储保护，
业务回答/环境恢复不清除暂停。取消请求幂等，停止与保全完成后才关闭归属核实的开放 PR；
关闭失败保留占用，已经合并则保存合并事实，不关闭、不回滚。仅 closed 或非空 test merge
SHA 不释放；新鲜精确合并事实可释放 0a 队列，但不把业务置为 Done。

现有 localhost 控制 API：

- `POST /api/requirements/{id}/pause`、`POST /api/execution/pause`：`{"pause":true|false}`。
- `POST /api/requirements/{id}/cancel`：保存明确的取消及关闭授权。
- `GET /api/requirements/{id}/delivery`：精确身份、验证 ID、consumer、PR、动作和尝试记录。

写接口沿用 Origin/CSRF 保护。恢复条件不满足返回 409。取消的 HTTP 回应只确认意图；
最终结果以持久化 cleanup_complete 和队列所有者为准。

## 真实 A01 入口

`node tools/a01-smoke.mjs INPUT.json NEW_EVIDENCE_DIRECTORY` 使用现有 Material 表单创建
Draft、评审 Ready，保存截图后关闭离线浏览器，通过后台交付观察接口等待真实 PR。
它不会配置授权、注入伪造验证结果、创建测试替身或自动取消超时任务；超时后继续对账原需求。
该入口有真实写副作用，不加入默认测试。

部署先提供现有 `GITHUB_APP_CONFIG`、`RUNTIME_CONFIG`、独立 PostgreSQL、执行目录和
GitBroker canonical bundle。Runtime 配置的 `preparation.baseline` 是该授权单仓 bundle
中明确的完整基线 SHA；`preparation.launcher`、预检 adapter 和 custom validation Plan
沿用现有配置。没有固定 baseline 不领取新的 Ready。首次准备把 Run/worktree 意图先落库，
通过现有真实预检后，领取和绑定 worktree 同事务进行。重复 tick 不重建 Run。
首次准备遇到跨 incarnation 的未知遗留项会阻塞并保留原路径，需先核对旧准备事实；
不会猜测可重跑。此项不承诺自动更新 canonical 到新 main 或多仓初始化。

启动配置好的 API 和 Angular 代理（见 local-development.md），登记已授权的验证仓。
输入文件例子中每个值必须换成此次授权的真实值：

```json
{
  "origin": "http://127.0.0.1:4200",
  "repository": "OWNER/DESIGNATED_TEST_REPO",
  "authorized_repository": "OWNER/DESIGNATED_TEST_REPO",
  "repository_id": 123,
  "title": "A01 single-repository smoke",
  "description": "实现此次已授权的小改动",
  "acceptance": "指定测试通过",
  "selector": "approved_test_selector",
  "expected": "Exit 0",
  "timeout_seconds": 900
}
```

`result.json` 使用 `codexsymphony-a01/v1`，记录 real/incomplete/submitted、时间、
repo、Requirement/revision、Ready 响应、validation_id、action_key、attempt、
PR URL/number/head 和 consumer；`review.png` 保存实际浏览器评审画面。
与服务器 Run/验证原始日志和 delivery_observation 一起保存，交付记录不替代精确提交双 Gate。

GH-21 开发测试中的脚本化 provider、Fake Remote、数据库故障注入只是确定性测试。
真实 Runtime 协议 smoke 也不等于真实 A01。GH-24 的操作员已提供授权单仓部署和具体
Contract，真实浏览器创建的 Requirement 1 已 Ready，但存储故障中断，真实 PR 尚未产生。
使用 `--resume PREVIOUS_RESULT.json` 只读续观原需求；部署修复与完整证据见
[单仓验收](single-repository-acceptance.md)，不借用开发宿主 PR #40 或开发 PR 充当产品验收。

本次开发命令、真实本地 Git/HTTP 测试和源码质量测量见 [GH-21 验收记录](quality/gh21/README.md)。

### 已耗尽发布尝试的人工恢复

部署问题修复后，可向现有 `/api/requirements/{id}/operations` 提交当前 `version`、
唯一 `request_id` 与 `action: "delivery_recheck"`。它只接受 Running 需求的当前 revision、
未释放且因暂时网络失败/写入额度耗尽而 blocked 的 publish；身份冲突不能由重检清除。
迁移 0015 的 `attempt_limit` 增加一个三次写入组，原 `attempts`、每次外部回执和候选身份
不变。版本与幂等校验、business event/request 和 operator intervention 记录授权；重复请求
不会重复增加额度。暂停、撤销、存储和验证授权仍在真正发送前检查。
产品先回读远端分支/PR，再从实际阶段继续；已成功 push 时只创建 PR，不重新编码或重推。
关闭动作仍维持原三次上限，本入口不解除取消状态。

App 服务通过 Git 推送时仅继承服务环境中的 HTTP(S)/ALL/NO_PROXY（含小写形式），
不继承其他 Git 配置、credential helper 或无关服务凭据。代理值留在部署环境，不能写入源码。

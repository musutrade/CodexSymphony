# 交付扩展接入（GH-121）

统一协议仍是 [P9](extension-protocol.md#p9-受控环境验证与交付)。
`delivery_extension::invoke` 是通用调用边界；`Adapter` 只暴露
`capability_check/submit/observe/reconcile`。具体实现的 Input 是冻结的任务、候选、
目标和操作类型，不能接收 Agent 任意命令或 API 请求。P9 实现使用 `Request::check`
核对原 FrozenConfig、ControlledConfig、部署登记、目标和操作引用。
`Control` 复用调用方原有的授权、验证、预算、意图、监督及证据台账，未增加第二状态机。

## 调用顺序与 GitHub 迁移

写入顺序是 check → 完整验证及环境准入 → before_deliver → 再次准入 → 持久化意图
→ 适配器 submit → 保留响应或错误 → 核对结果身份 → 适用的交付后验证。
读操作不受暂停后禁止新写的规则阻断，仍可 observe/reconcile 原操作。
结果即使错身份或未知也先保留；写响应不能直接产生 Submitted/Done 或 CI 成功。

既有初次发布和修复更新都经 `github_publication_adapter`；取消关闭复用同一边界，
但不要求取消后的原验证重新获得发布授权。适配器在关闭前重新读取 PR 并保留合并事实。
`github_merge_adapter` 经同一通用边界发送合并，并在附加 Hook 后重新读取能力、检查、
head/base 及 PR 可合并性；GitHub 的条件合并和保护规则继续生效。
原 delivery/merge_operation、attempt、receipt、观察和清理记录保持权威。
合并后完整验证继续由原 `merge_acceptance` 在独立观察到精确 merged SHA 后执行，
不是在收到 merge 响应时宣告完成。Requirement、AgentRun、GitHub 事实与验证证据不合并。

未知 push/create/close 不因一次负面远端读取而重发。只有原记录明确包含服务端拒绝
（401/403/404/409/422）才允许沿用原预算产生后继尝试；缺响应、服务端未知或已成功
响应只能对账原操作。原预算计数不会清零，操作员重查也不能消除未知意图。

## 附加 Hooks

环境 Plan 的 `controlled.extensions` 可选择包含 `before_deliver` 的受审登记；
部署环境 Profile 的可选 `extensions` 保存对应登记白名单。没有额外登记时 profile
摘要保持旧算法。增加登记改变 profile 摘要和完整方案身份，必须重新评审和批准，
不会替换旧任务的冻结配置。

`DELIVERY_HOOK_REGISTRY` 指向部署控制的 JSON 数组，每项包含：

- `registration`：与冻结 Plan 和宿主 profile 完全一致的登记。
- `stages`：`before_deliver`、`before_publish`、`before_merge` 的适用集合。
- `plan`：复用受审验证进程 Plan（entry、entry_sha256、steps）。

登记的 implementation_digest 等于 entry_sha256；config_ref 等于对
`[stages, plan]` 的紧凑 serde_json 序列化取 SHA-256。它同时绑定适用时点与命令参数，
删除某时点不能绕过冻结审核。credential_provider_ref 必须为空。实现、配置和证据
放在候选工作区之外；附加 Hook 不带交付凭据，不能替换完整验证或修改候选。
GitHub before_publish/before_merge 的选择和调用留在 GitHub 适配层。

调用输入复用 P9 Call，绑定原候选、环境、任务、策略、操作及实现。原
project_hook_invocation 保存不可替换的 input；操作和时点分别占用独立 invocation。
重复调用只读取原始证据。输出丢失、重启或非幂等未知调用不会换新 ID 重跑。
结果与原始日志、实现、候选和完整静止回执一起核对。输入上限 64 KiB，每个捕获流
上限 1 MiB；继承原验证期限，使用同一 subreaper、停止协议及存储心跳。
四类生命周期 Hook 的事件、配置和行为不变。

## 凭据与无 GitHub 项目

`github_credentials::FileProvider` 在 GitHub 扩展部署边界读取绝对常规文件中的 App
私钥，交付输入不携带秘密。AppClient 继续限制安装、仓库、权限和 API 路径。
带凭据的实现是固定原生适配器；不会用凭据启动候选脚本。需要可信 CheckRun 时，
发布 App 与被信任的 Checks App 必须不同。无秘密的适配器不需要提供空凭据文件。

Repository 可显式选择 `delivery: "local_git"`，省略 github_repository_id，remote
为受审本地目标引用；配置缺省仍表示历史 GitHub PR 路径。仅本地项目的部署不加载或
探测 App 配置。更改已登记仓库的交付身份须新建登记，不能热切换历史任务。
通用接口无 PR/CI 必填值，受控测试使用无凭据本地适配器覆盖全部四种操作。
实际 local_git 运行与写入实现属于 #105；当前未安装时明确阻断，不伪造本地交付完成。

## 迁移、验证与回退

迁移 0034 只为原 Hook 调用表增加 nullable input。旧 Hook 记录和累计预算保留。
配置未选择额外 Hook 时不会执行它。旧任务继续使用原策略，现有 GitHub 写入口仍需
经过核心完整验证准入和原事务授权。绝对私钥路径是部署提供方的要求；旧相对路径
需在部署配置中显式迁移，不能从候选工作区解析。

回退先暂停新领取、确认进程组静止、保全材料并对账未知远端操作。保留 0034、调用
input、attempt 和所有结果，使用能识别新准入及未知写规则的兼容版本。不得回滚到
把未知请求当作可重试的旧 worker，也不能删除未知 Hook 记录来重启非幂等脚本。

受控覆盖见 delivery_extension、delivery、automatic_merge、environment 的 P9 Hook
场景和 github_credentials 测试。真实 GitHub 使用宿主凭据隔离 runner；基线 PASS
不是本次候选验收。候选实际回归、完整检查与 Gate 状态以 artifacts/gh121 的证据
及 `.symphony-evidence.json` 为准；本文不声明生产部署或 #105/#122 已完成。

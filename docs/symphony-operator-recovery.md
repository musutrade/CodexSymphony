# 开发 Symphony 的等待与恢复

本次改动作用于 `/home/gem/symphony/elixir` 开发控制器，不改变 Rust 产品的
Requirement/AgentRun/PR 状态机，也不提供强制清空产品 owner 的入口。

## 状态和预算

- `waiting_external`：保存外部操作请求，继续占据串行队列，不启动模型轮询。
- `infrastructure_wait`：已确认的 before_run 环境失败、响应超时或上游重试耗尽。
  默认最多 3 次基础设施失败，前两次按 30 秒、120 秒等待后重试；上限由
  `agent.max_infrastructure_attempts` 控制。显式恢复增加额度后，退避最多 300 秒。
- `blocked`：代码/连续错误/基础设施预算耗尽或需要明确操作的阻塞。
- 每次实际 worker 启动仍计入 `run_attempts`；基础设施失败和外部等待单独计数，
  不占用代码尝试预算。token、运行时间和 CI repair 次数不归零。
- 台账写入失败暂停调度、停止 worker，并保留内存中的待落盘记录；恢复写入后
  保持 blocked，需受审计恢复。若宿主在落盘失败期间掉电，未持久化用量仍可能丢失。

## 宿主恢复 API

`POST http://127.0.0.1:4011/api/v1/GH-N/recovery`，必须来自 loopback、没有 Origin
头，并带宿主 `SYMPHONY_OPERATOR_TOKEN`。配置由 `server.operator_token_env` 引用。
令牌保存在 workspace 外，agent 的 bwrap 环境不挂载、不继承此令牌。

请求必须恰好包含：

```json
{"action":"resume","request_id":"operator-fix-unique-id","revision":0,"reason":"已修复的具体原因","evidence":"宿主非秘密证据路径"}
```

`action` 可为 `resume`、`retry_infrastructure`、`external_completed`。
先从 state 的 waiting 条目读取 recovery.revision；过期版本返回 409。
相同 request_id 和相同正文重放返回原结果；换正文重用 ID 返回 409。
恢复记录保存原预算、前状态、外部请求、证据和时间。不绕过 token/运行时间硬上限。
当前 API 支持本机 workspace；远程 worker 恢复明确拒绝，避免在错误主机移动文件。

宿主 CLI 使用正文文件，避免命令行泄露凭据：

```sh
python3 ~/.local/share/codexsymphony/symphony/operator/operator_bridge.py \
  --issue GH-N --command /absolute/non-secret/recovery.json
```

## 外部操作桥接

`python3 tools/install_symphony_operator.py` 安装独立 systemd timer，每 15 秒检查
外部请求。它只经 API 恢复，不修改 handoff journal。安装后需在维护窗口重启控制器
以加载令牌；日常恢复不用重启服务。

`~/.local/share/codexsymphony/symphony/operator/grants.json` 是宿主授权来源：

```json
{"version":1,"grants":[{"issue":"GH-N","revision":1,"operation":"product.deploy","request_id":"deploy-1","request_sha256":"完整 external 对象的规范 JSON SHA256","argv":["/absolute/host-owned/pinned-operation"],"executable_sha256":"执行文件 SHA256","timeout_seconds":120}],"profiles":[]}
```

规范 JSON 为 Python `json.dumps(request, sort_keys=True, separators=(',', ':'))`。
固定 argv 只能由宿主写入；部署脚本自身须固定输入版本、校验身份、保留部署前后
owner/预算/存储/恢复记录并留下非秘密证据。不能用可变 workspace 脚本作宿主入口，
也不要用通用解释器加未固定脚本来规避 digest 绑定。新部署需准备具体 grant，
不能自动把任意 agent 请求当授权。已有 grant 自动执行，无需用户重复确认。

安装器仅预授权一个只读 profile：`product.identity_preflight`。它检查当前 3081
部署健康、数据库和 disposable/disposable-2 的固定 GitHub ID 与 delivery_ready。
它不合并 PR、不取消 Requirement、不证明 Runtime 或真实 A13 验收。
该 profile 的 resume_condition 必须精确为
`health=ok; database=ok; repositories=1360824360,1377749969; delivery_ready=true`。

执行前落盘 started receipt，退出成功后记 succeeded，再调用幂等恢复 API。
API 响应丢失只重试确认，不重跑操作；进程在操作期间崩溃留下 started，不盲目重放
可能已完成的部署。失败/超时保存失败 receipt，不自动把失败当完成。输出不进日志，
执行脚本自行保留非秘密验收证据。receipt 存在上述目录的 receipts/。

## 当前部署和验证

开发模型保持 `gpt-6-astra low`；每个 worker 最多 40 turns，token、运行时间、代码
尝试上限保持原值。正式 Harness-Gate 的 App、PR/head 绑定和合并条件没有放宽。
新测试覆盖 OTP 重启、独立基础设施上限、外部等待、恢复幂等/版本冲突、磁盘写失败、
宿主授权匹配与执行后断连恢复。实际部署结果见本次执行记录。


### 2026-09-20 部署记录

开发控制器提交 `588686d`（本地分支 `fix/operator-recovery`）已构建并部署。
`make all` 全部通过：357 项测试，0 失败、6 跳过；既定覆盖率门槛和 Dialyzer
通过。宿主 Python 工具 26 项测试通过，仓库 Gate 配置校验通过。
这不是新增产品功能或完整远端 Harness-Gate 验收。

部署后控制器、产品 API/Web、operator timer 和 fixture cleanup timer 均运行正常。
恢复 API 未鉴权返回 403，宿主鉴权后的未知 Issue 返回 404；未对已完成 Issue
发起恢复。部署前后 handoff 台账 SHA256 一致，未清空预算或执行记录。
只读产品预检曾遇到能力观测的 60 秒有效期边界，增加最多 4 次、间隔 5 秒的读取，
总操作仍受 120 秒超时约束；实际身份预检最终通过。

备份及验证记录：`/home/gem/.local/share/codexsymphony/symphony/operator-deployment/20260920T034909Z`。
其中包含原 workflow、journal、service unit、测试日志和 `verification.json`，不含令牌。
新宿主部署的固定授权需登记到 grants；现有只读身份 profile 自动处理匹配请求。

### 本次仓库收尾

恢复 API 客户端已增加拒绝 HTTP 重定向，避免令牌转发或恢复请求重放；28 项宿主测试通过。
该补充尚未部署，不能沿用上述旧部署结果声称运行服务已包含修复。
本批次交付边界、0a 证据复核与下一项工作见 [收尾记录](quality/development-closeout/README.md)。

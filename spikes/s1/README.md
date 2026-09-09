# S1 spike — Codex app-server dynamicTools 与 workspace-write 沙箱边界

日期：2026-09-07　　codex-cli 0.153.4　　Linux 7.0 / landlock　　结论已写回综合方案 23.S、8.1、8.3、17.3

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | `dynamicTools` 能否注册、调用、回包 | **能。** `initialize.capabilities.experimentalApi=true` → `thread/start.dynamicTools=[{type:"function",name,description,inputSchema}]` → 模型调用时服务端发 `item/tool/call` 请求（params: `threadId, turnId, callId, tool, arguments`），客户端回 `{success, contentItems:[{type:"inputText",text}]}`。两次真实运行均按顺序收到 `create_local_commit` → `report_completion`。 |
| 2 | 沙箱能否阻止 Agent 直接写 `.git` | **能，但取决于 app-server 的启动 cwd。** workspace-write 的可写根 = **app-server 进程 cwd** + `/tmp`；`command/exec` 或 `thread/start` 的 `cwd` 参数**不**增加可写根；只有每个可写根顶层的 `.git` 受保护。若 app-server 从仓库上层目录启动，整棵树可写，嵌套 worktree 的 `.git` 不受保护。 |
| 3 | git worktree 场景 | **理想。** 以 worktree 为 app-server cwd：文件可写；`.git` 指针文件只读；真实 git dir 在 canonical clone 的 `.git/worktrees/<name>`，不在可写根内 → `git add/commit` 以 `index.lock: Read-only file system` 失败；`git status/log/diff` 正常。真实模型运行中直接 commit 失败、`create_local_commit` 成功，host 侧 git log 只有平台创建的 commit。 |
| 4 | 沙箱是否限制读 | **不限制。** Agent shell 可读 `~/.codex/auth.json`、`~/.ssh/`、`/etc/passwd`。workspace-write 只是写保护。 |
| 5 | 网络 | `-c sandbox_workspace_write.network_access=false` 下 curl 返回 rc=7（连接被拒）。注意用户 config.toml 里全局是 `true`，平台必须每次启动显式覆盖。 |
| 6 | `approvalPolicy="never"` | 未收到任何 `*/requestApproval` 请求；沙箱拒绝直接作为命令错误返回给模型，模型据此报告并继续。 |
| 7 | token 事件 | `thread/tokenUsage/updated` 每 turn 3～4 次，独立于 `turn/completed`。 |
| 8 | 模型对工具描述的解读 | 描述里写"这是唯一提交方式"时，模型把它当禁令，拒绝执行提示词里明确要求的直接 commit 测试。工具描述要写中性事实，禁令放在 developer instructions 或由沙箱强制。 |

## 对方案的修正

- 8.1 / 21.1：**每个 Run 一个 app-server 进程，以该 Run 的 worktree 为 cwd 启动**，并显式传 `-c sandbox_workspace_write.network_access=false`（或按 Policy 白名单）。不能用一个常驻 app-server 服务多个 Run。
- 8.1：`workspace-write` 不等于 `.git` 可写这一句成立，但前提是上一条；文档要把"cwd = worktree"写成硬性要求。
- 17.3 / 0a 边界：0a 单进程形态下 Agent 能读到运行平台的用户的所有文件（含 Codex 认证、SSH 私钥）。0a 只接管自己可信的仓库；0c 的独立 UID executor 不是可选加固，而是接管任何非自写代码的前提。
- 8.3 验收第 7 条（"沙箱直接写 .git 被拒绝"）已通过；第 8 条（重启后声明不丢失/不重复交接）留给 0a 实现时测。
- S1 退路方案（平台在 turn 结束后全量提交）不需要启用。

## 复现

```bash
cd spikes/s1
python3 s1_dynamic_tools.py      # 真实模型 + worktree + 两个动态工具，约 1～2 分钟
python3 s1b_sandbox_probe.py     # 不经模型，用 command/exec 探沙箱边界
```

`s1_dynamic_tools.py` 会在同目录创建 `canon/`（canonical clone）与 `ws/`（worktree），并写 `s1.log`（完整 JSONL 收发）。
运行 `s1b` 前注意：它探到的边界取决于你启动它时的 shell cwd（见结论 2）。

## 未验证

- macOS（seatbelt）行为，仅测 Linux landlock。
- `thread/start` 的 `cwd` 与 app-server cwd 不同时，模型工具（非 command/exec）的可写根是否也只看进程 cwd——S1 两次模型运行 thread cwd 都等于或位于进程 cwd 之下，未单独区分。0a 实现时两者取同一值即可回避。
- 多 turn 长会话下 dynamicTools 回包顺序与并行调用。
- `turn/interrupt` 后 `item/tool/call` 的挂起请求如何终结（8.3 验收第 4 条）。

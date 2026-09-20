# GH-61 / M1-02 验证映射

前置 PR #60 已合并，开发基线为 `b7017ac626f9b32a65caf21706b0dd7ced572af2`。
范围仅为自然语言生成、同一 Draft 编辑/保存及生成记录。不是整组执行授权或 V1 产品验收。

| AC | 实际检查与证据 |
| --- | --- |
| AC01 | `artifacts/gh61/real_browser_acceptance.py` 在独立 PostgreSQL/API 上由真实桌面/手机表单触发模型；小需求一项、大需求至少三个 code_change 加一个 validation_only，保留完整生成记录与 Draft。记录与截图保存在 `artifacts/gh61/real-browser/`；与 `runtime_real` / scripted provider 测试分开标记。 |
| AC02 | 原 `drafts.rs` 的迁移/重启/完整读回回归；`generation.rs` 生成事务和版本；`e2e/generation.spec.ts` 修改、删除、重排、保存、重开及稳定 ID。 |
| AC03 | `invalid_output_never_partially_writes_a_draft`、原 Draft parser/API 负向矩阵；并发同请求去重和生成/编辑 CAS 冲突，失败当前表与历史表均无部分写入。 |
| AC04 | 独立持久状态、同输入幂等、启动恢复标记；真实 pinned Runtime + 明确标记的本地 provider 测试超时、失败、token 超限和缺失用量；比较 Run/预算/owner 前后不变。 |
| AC05 | 桌面 Chrome / Pixel 7 的真实 API 浏览器流程、键盘提交、axe 全规则、横向溢出检查和截图；生成消耗与执行授权分开说明。 |

## 验证边界

本地检查不是正式签名 Gate。精确 PR head 的 Harness-Gate 与 Trusted Harness-Gate
由宿主执行；没有修改门禁策略、可信摘要或阈值。脚本化 provider 只验证协议和错误路径，
不冒充 AC01 的真实生成。

预检发现固定 3081 端口已占用；后续浏览器检查使用新 API 的随机端口和独立 schema。
复用测试数据库时一次覆盖率运行触发旧预算场景的存储保护；保留失败日志，并通过提供的
`dbctl.py recreate test` 重建独立一次性夹具后继续。没有操作生产数据库、服务、owner 或预算。

`.symphony-evidence.json` 声明工作区证据文件及 SHA-256。宿主归档回执不由 Agent 伪造。
A13 历史原件缺口仍按已有 [恢复索引](../gh25/evidence-recovery.md) 如实保留。

## 真实调用与使用证据

最终真实浏览器调用：桌面小需求输入 6831 / 输出 216 token；手机大需求输入 6877 / 输出 882 token。
两次调用均报告缓存输入 4352 token（已经包含在输入内），计时分别为 13 / 31 秒。
保留 Runtime 0.154.0 的实际 user-agent、thread/turn ID、设置摘要、原始输出和保存后的 Draft。
随后真正结束并重新启动 API，完整读回相等；零 AgentRun、零编码预算记录、owner 为空。

`artifacts/gh61/contract/observations.json` 包含 74 个实际 operation/status 响应，含一次性测试数据库
临时拒绝连接得到的真实 health 503，随后已恢复连接。固定契约插件测量 breaking_changes=0、
client_drift=false、compatible=true；另验证两条真实模型记录符合最终响应 Schema。
没有把合约 SQL fixture 或 mock 回应当作真实模型证据。

质量检查日志与结果保存在 `artifacts/gh61/`。此前一次 Rust 源码映射不支持元组表达式闭包的
入口锚点，已改为具名函数并由同一固定 collector 重新测量；未修改 collector 或其策略。
前端插桩使用工作区内临时副本，实际源文件不含插桩或 TypeScript 抑制注释。

## 发布前检查

- `python3 /opt/symphony-env/run.py cargo test --workspace --locked`：114 项通过，包含真实 PostgreSQL、Runtime、暂停/取消、唯一 owner、预算和持久化回归。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `npm run lint`、`npm test -- --watch=false`、`npm run build`：通过；前端 25 项单元测试。
- `npm run test:e2e`（独立 schema、随机 API 端口）：桌面/手机共 20 项通过，日志 `artifacts/gh61/browser-final-2.log`；另有两项真实模型浏览器验收通过。
- `cargo llvm-cov --workspace --locked --json` 与固定源码 collector：1049 个 Rust 函数，覆盖率/CRAP 违规为零；前端原源码插桩测量 62 个函数，违规为零。仍使用行/region 或行/函数覆盖率至少 80%、CRAP ≤ 10 的原阈值。
- `python3 tools/gate.py config check`、`secrets --json`、`audit --json`：配置有效，无秘密或架构违规。

最终命令、退出码及日志见 `artifacts/gh61/release-checks.json`；前端脚本日志为
`frontend-release-{lint-2,unit,build}.log`，真实调用日志为 `real-browser-acceptance-2.log`。
浏览器测试曾因新测试依赖另一测试先登记仓库而失败；已改为自行准备仓库，保留失败日志。
最终复核启动脚本还遇到管道读缓冲遗漏就绪日志，改用日志文件读取后 20 项全部通过；
该脚本位于 artifacts，不是产品运行路径。两次尝试均保留记录。
恢复测试同时验证已观测 token 数保留且中断用量标记不完整。

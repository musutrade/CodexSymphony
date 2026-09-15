# 开发环境远端验收（2026-09-15）

环境链路已完成：独立宿主采集并发布可信检查，模型经 Elixir 宿主 Git Data API 创建 PR，
模型退出后由控制器等待 CI、按精确 SHA 合并并关闭 Issue。Agent shell 未使用 git push。

| 验收对象 | 结果 |
| --- | --- |
| [准备 PR #26](https://github.com/musutrade/CodexSymphony/pull/26) | PR 与合并后 main 完整 PASS |
| [环境 Issue #27](https://github.com/musutrade/CodexSymphony/issues/27) | 控制器于 06:23:19 UTC 自动关闭 |
| [模型发布 PR #28](https://github.com/musutrade/CodexSymphony/pull/28) | 两个必需检查 PASS，06:23:15 UTC 自动合并 |
| [PR #28 门禁](https://github.com/musutrade/CodexSymphony/actions/runs/34936449798) | run-8a55d2d93bfc，3 个 producer、20 条 evidence |
| [合并后 main 门禁](https://github.com/musutrade/CodexSymphony/actions/runs/34936704483) | 完整 PASS |
| 实际嵌套 command/exec | PASS；Core 0.4.5，Git 元数据写入返回 EROFS，两个宿主敏感路径不可见 |

[acceptance.json](acceptance.json) 保存精确 SHA、检查发布者、run/attempt、报告摘要、
控制器最终预算与保留的失败记录；[main-protection.json](main-protection.json) 是保护规则回读。
main 强制 `Harness-Gate`（App 15368）及 `Trusted Harness-Gate`（App 4867361），
strict 更新、管理员强制、线性历史、禁止 force push 和删除。阈值仍为 CRAP ≤ 10、
覆盖率 ≥ 80%；Rust 使用独立源码分支复杂度插件，MIR 只作诊断。

## 修复和证据边界

首次模型尝试被系统 bwrap 的 AppArmor 子进程 profile 阻止；管理员安装项目专用外层
启动器/profile 后，在同一工作区恢复。原尝试和消耗保留，最终共 2 次尝试，台账 phase=done。
系统 bwrap 与全局 sysctl 未修改，Codex 内层 workspace-write 继续启用。

恢复后的模型会话首次配置检查仍因旧 PATH 选中 Core 0.4.2 而失败，显式选择锁定版本后通过。
该事实保留于模型提交的 [symphony-e2e.md](../symphony-e2e.md)，没有改写为首次成功。
安装的后续启动器已修正默认 PATH；`python3 tools/symphony/check_sandbox.py` 通过真实
app-server command/exec 单独验证默认 Core 0.4.5 与隔离。#28 的会话并未重新启动来验证新 PATH。
首次准备 PR 的旧 Actions attempt 因部署重启而中断，也保留为失败，后续新运行独立通过。

本目录仅保存可公开的结果与定位信息。完整测量原件位于本机
`~/.local/share/codexsymphony/gate-host/runs/`，远端宿主回执在 `remote-gate/jobs/`，
交接台账在 `symphony/WORKFLOW.lifecycle.md.handoffs.json`；未收录认证文件或私钥。

## 0a 队列

现有 #12–#25 共 14 个 Issue 均仍为 open，保持原 `symphony-ready` 标签。
独立部署额外要求 `symphony-environment-acceptance` 标签（AND），所以没有领取它们。
后续先核对 #12 与准备 PR #26 的实际覆盖及剩余验收项，再按 #12→#25 的依赖顺序推进。
本次文档任务没有验证产品业务、真实 spikes 或宣告任何 0a Issue 完成。

# 最终发布 Gate 前置阻断

候选 `e8e47d226d9b48e91600aa2d32fc4fc33ab4cc94`（树
`1c6bd8564ad340aeba51ced5ae0adef499357153`）已完成源码测量和前置检查，
并完整复制到独立 Git 仓库 `/home/gem/.local/share/codexsymphony/workspaces/GH-106`。
隔离环境内 Git 元数据可读，树一致，启动前专用磁盘可用空间超过 20 GiB。

调用已安装 `/home/gem/.local/share/codexsymphony/publication/guard start --issue GH-106`
在启动前返回 `REJECTED: environment drift: workspace contract differs from installed host release`。
服务无活动 PID，没有产生完整 Gate PASS，也没有推送分支或创建 PR。

已确认差异仅为发布宿主 release `50393b9960f1ead0` 的 Codex 版本及二进制摘要：
它仍绑定 `0.156.1`，当前 main、功能候选及已批准 Gate 宿主 `b4c9c09b96c53b3d`
绑定 `0.157.1`。`environment_contract.py` 实现本身一致，不能通过回退仓库锁来消除拒绝。
原始记录保留在 `/home/gem/.local/share/codexsymphony/gh106-acceptance/final-gate-preflight-rejection.json`。

[AGENTS.md](../../../AGENTS.md)规定：
“ A disagreement with installed approval is a concrete prerequisite failure. ”
并明确开发流程不授权修改受信宿主内部。因此本次没有运行安装器、重写批准或改动宿主。

恢复需要由受信宿主维护流程完成经审查的发布宿主更新，使其匹配已经批准的当前环境契约，
保留原证据和账户。随后核对本地分支最新文档提交与验证副本的精确树，重新运行最终完整 Gate，
取得完整宿主回执后再发布 PR、读取远端提交树并建立正常 CI。若代码或测量环境发生变化，
先按开发流程重新测量；仅安装请求被接受不能证明 Gate 已启动或通过。

本文件是阻断交接说明，不是发布回执。线上付费模型验收的独立边界仍见本目录 README。

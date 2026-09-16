# GH-19 人工接手记录（2026-09-16）

第四次自动运行的阻塞事实保留：独立的 Python 命令探针不等于 Rust Runtime 验收。
本次人工接手保留原 Runtime 代码、四次历史和累计 token/时间，不再通过追加自动尝试绕过问题。

## 已实现

- 固定的 `runtime-product-acceptance` 宿主操作不接受 Agent 提供的命令、路径或配置。
- 操作者从指定 GH-19 源码构建并安装独立 Rust 测试和 supervisor；安装记录绑定完整源码清单、SHA256 和二进制 SHA256。新增、删除、修改或链接源码使验收入口拒绝执行，须重新审查安装。
- Rust 测试运行在独立、无外部网络的外层沙箱；实际锁定 Codex 0.154.0 app-server 再按产品策略执行命令。使用空白临时 CODEX_HOME，不复制或链接共享认证；真实握手、动态工具往返、interrupt 和 ECHILD 退出证明，使用本地协议 fixture，模型调用为零。
- 命令环境单独保留 Git 只读、凭据隐藏、允许域/拒绝域/直连阻断实测。原嵌套失败不改为成功。
- Runtime 使用固定 schema 生成的类型、持久化动态请求结果和互斥结束意图、分离并限额保存 stdout/stderr、预算记账与真实进程监督。
- 问题与答案独立于 RPC 生命周期；2h 等待和 8h Run 期限从持久化起点计算。
- 可配置 Runtime worker 处理已预检 Run 和持久化回答的恢复意图。旧进程组静止并保全后，恢复未完成代码到新 Broker worktree，执行独立预检，事务关联答案与新 Run，再执行 Runtime。原暂停、撤销、其他 blocker 与需求额度继续生效。
- 预检成功后因暂停未能启动时，短期复用仍有效的证据；过期证据重新实测并计入原有有界预检台账，不重置重试组。

## 验证边界

实际独立测试验证真实 pinned app-server 的传输和监督；完整 PostgreSQL Runtime client/工具/隔夜回答测试使用确定性 stdio fixture，不声称调用真实外部模型。
初始需求获取、验证到 PR 的下游功能仍分别受自身准入限制。本项不把执行完成视为验证通过、PR 合并或业务 Done。

完整签名门禁必须沿用 CRAP ≤10、覆盖率 ≥80% 与原可信采集路径。新增 Tokio process/io-util feature 改变 Cargo.toml 的采集管线摘要，需要操作者批准新测量系列；不得复制旧签名、修改原批准或假报 PASS。

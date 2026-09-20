# GH-25：第二私仓与严格串行验收

2026-09-20 后续复核：[证据恢复与发布索引](evidence-recovery.md)。12 项原清单已恢复
8 项且摘要完全匹配，4 项仍缺失；当前服务就绪异常与以下历史验收结果分开记录。

2026-09-20，在 GH-24 真实 A01 已交付、关联开发 PR #52 合并后，原 localhost 产品接入
`musutrade/disposable-2`（1377749969）。[真实 A13 记录](real-a13.json)绑定
Requirement 2/revision 1、Run `2a2ae1eb-e147-47f8-a650-8869e98a5e25`、独立验证、
候选 `3974acfce705c774d4590c72cd508a40c2f1ff58` 和
[产品 PR #2](https://github.com/musutrade/disposable-2/pull/2)。这不是操作员能力探针 PR #1。

## 串行与精确归属

首仓 Requirement 1 仍占用时，第二仓 Ready 的 Run 数与模型调用数均为零；首仓 Run
与五次历史调用不变。操作员随后审核并合并首仓 PR #4，产品观察到精确合并
`fbcad73d9b5f1155b44c79a59b828d9f2ae3bf09` 后正常释放，再自动领取原 Requirement 2。
首仓仍为 Submitted，累计预算未重置；没有直接改占用者或重建第二需求。

第二仓一次真实调用完成执行，独立验证成功，再由产品 App 创建 PR。PR 两端仓库 ID、
生成分支、默认分支 main、候选和 validation_id 已核对。GitHub 的 push/PR Actions
运行均成功，PR job 实际执行 `cargo test --locked`。产品只读观察曾显示过期，随后正常
刷新为 `stale=false`；当前第二 PR 未合并，需求保持 Submitted。

两仓复用原全局 8 GiB 存储预算，第二 Run 的资料记录独立 repo/requirement/revision/Run/
candidate/PR 身份。部署前后 20 张执行、预算、恢复和存储表逐项保留；原服务曾有存储
guard，操作员核对后执行了一次有审计的 `storage_recheck`。此部署不声称零人工介入。
第二需求自己的指标为一次模型调用、零代码修复、Ready 到 PR 820 秒（包含等待首仓）。
活动资料保护、跨身份归并拒绝、容量准入和中断回收继续由真实文件/数据库集成测试覆盖；
未为演示强制删除仍在使用的产品资料。

私仓只授予所选 Checks/Actions 能力时，旧观察器还读取无关 legacy Statuses，导致 403。
本次修复仅在策略显式选择 Status 来源时才读取它；显式 Status 的权限失败仍阻塞。
开发 GitHub 服务的 Checks 读取也返回 403，已保留原响应；独立产品 App 的就绪与新鲜
观察、GitHub PR/Actions 的实际读回分别记录，不从开发凭据推断产品权限。

## 回归与发布边界

[A01–A12 索引](../../single-repository-acceptance.md)和
[GH-24 真实证据](../gh24/README.md)继续有效。新增 `multiple_repositories` 测试核对
登记、冻结版本、缺能力零 Run、占用时不切仓、缺路由不绕队首及单仓配置拒绝第二仓；
`delivery` 测试覆盖选择到领取之间队首变化，`legacy_contract` 保持原闭合 API 兼容。
存储回归来自 Rust 产品 `storage.rs`，不引用开发宿主 PR #40 作为 A10 通过证明。

前端沿用 arc-admin 共享样式，20 项单测及 16 项桌面/手机浏览器测试通过，包含键盘和
axe；没有为本项增加远程手机接续、并行调度或自动 CI 修复/合并。
[桌面截图](repositories-desktop.png)和[手机截图](repositories-mobile.png)来自确定性浏览器夹具。
完整 Cargo workspace（含真实 Runtime）、fmt、Clippy 和原生源码质量结果见本目录的
本地验证与测量记录。开发采集不签发正式 Gate。

原始证据及摘要见 [保留清单](retained-evidence.json)，大体积原生对象、profiles、浏览器
产物保留在 `artifacts/gh25/`；操作员原服务资料仍在其受信部署中。正式精确提交的
Harness-Gate / Trusted Harness-Gate 由独立服务验证。第 24 章仍不在此提前宣布完整
0a 发布；当前能力是 localhost 内部里程碑，产品交付终点为 Submitted/PR。

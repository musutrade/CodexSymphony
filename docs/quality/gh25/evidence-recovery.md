# A13 证据恢复与发布索引（2026-09-20）

原保留清单的 **12 项已恢复 8 项，4 项仍不可用**。恢复件均与合并前
[原清单](retained-evidence.json) 的 SHA-256 完全一致，原清单及
[历史 A13 摘要](real-a13.json) 未修改。此次没有启动模型、创建需求、合并产品 PR、
改写产品数据库或重置预算。

## 已恢复文件

| 原文件 | 可读副本 | 来源 |
|---|---|---|
| operator-deployment/preservation.json | [部署保全](recovered/operator-deployment/preservation.json) | 宿主 deployment-1 中的原件 |
| operator-release/after.json | [首仓释放](recovered/operator-release/after.json) | 宿主 release-1 中的原件 |
| a13-occupied/summary.json | [占用期间记录](recovered/a13-occupied/summary.json) | 从已提交 real-a13.json 的 occupied_queue 字段恢复，原摘要匹配 |
| a13-submitted/a13pr.json | [原 PR 响应](recovered/a13-submitted/a13pr.json) | 历史工具输出中的完整 GitHub 响应 |
| a13-submitted/a13checks.json | [原 Checks 响应](recovered/a13-submitted/a13checks.json) | 历史工具输出；保留当时 403，不改写成成功 |
| a13-submitted/a13actions.json | [原 Actions 响应](recovered/a13-submitted/a13actions.json) | 历史工具输出中的完整 GitHub 响应 |
| a13-submitted/a13jobs.json | [原任务响应](recovered/a13-submitted/a13jobs.json) | 重新只读查询同一运行，规范序列化后与历史摘要一致 |
| a13-submitted/second-delivery.json | [交付记录](recovered/a13-submitted/second-delivery.json) | 当前只读接口返回相同交付记录，按原格式序列化后摘要一致 |

[恢复清单](evidence-recovery.json) 保存原路径、原摘要、恢复来源、历史输出行号与记录摘要、
宿主备份位置及仓库内可读位置。按原摘要确认后才接纳恢复文件，不从不完整输出猜补内容，
也不执行历史会话中的命令。恢复件已进入 Git，后续删除开发 workspace 不再删除其唯一副本。

宿主另有副本：
`~/.local/share/codexsymphony/a13-gh25/evidence-recovery-20260920/`。
从历史输出或已提交摘要恢复的文件不冒充“从原目录找到的文件”；其内容一致性由原 SHA-256 证明。

## 四项缺失与补充证据

| 尚未恢复的原文件 | 可核实的事实 | 不能替代的部分 |
|---|---|---|
| repositories.json | 原 PR 两端仓库 ID、新的双仓登记读取、历史 A13 摘要 | 当时完整的仓库就绪响应 |
| first-operations.json | 历史首仓占用记录、释放原件及当前五次调用记录 | 当时完整时间线与全部指标响应 |
| second-operations.json | 原交付记录、精确候选与验证 ID、历史摘要及当前读取 | 当时完整 operations 响应 |
| second-operations-latest.json | 历史摘要包含原 Run、验证、存储身份和新鲜观察；当前同身份读取 | 当时完整 operations 响应及新鲜度快照 |

已搜索 `/home/gem` 下相应文件名、宿主部署和工作区目录、相关 `/data` 归档目录、
GH-25 Git 树及保留会话输出。没有找到上述四项与原摘要匹配的完整副本；这不是原件永久
丢失的证明。它们继续列为 unresolved，不删除条目、不改变旧摘要、不将新快照标为原件。

[本次只读复核](evidence-readback.json) 是新观察：第二需求仍是 Requirement 2/revision 1，
Run、候选 `3974acf…`、独立验证成功记录、PR #2 两端仓库 ID 和分支一致；第一/第二需求
模型调用仍为 5/1，两仓共享 8 GiB 配额。产品 PR 保持 open/unmerged；GitHub 的非空
`merge_commit_sha` 没有被解释为已合并。Actions job 的 `cargo test --locked` 成功记录可读。
GraphQL 聚合检查读取失败，REST PR 与 Actions jobs 读取成功，权限边界如实保留。

## 当前运行状态与历史验收分开

本次 05:22、05:24 UTC 两次读取均看到第二仓 delivery_ready=false、PR 观察 stale=true，
总体 repository_ready/runtime_ready=false；产品 health/database 正常。
这是当前就绪检查的待诊断项，不能因历史 A13 曾经通过而当作当前服务可直接接新任务。
这些新快照不回写历史 PASS，也不替代当时的新鲜观察记录。

GH-25 开发 [PR #53](https://github.com/musutrade/CodexSymphony/pull/53) 的精确 head
`4ee49561f05a4e4f048457ead34ffc87187fa207` 已取得两项正式 Gate 成功并合并。
PR #54 后续恢复工具修复的两项 Gate 也已通过，并合并为 `4d73c7d`；这些事实只覆盖
对应提交，不能证明缺失的历史原件已经找回。本次没有宣布完整 Phase 0a 发布或勾选第 24 章。

## 工作区保全缺口与后续顺序

开发控制器 `588686d` 的启动流程会对终态且无未完成交接的 Issue 调用工作区删除；
`remove_local_workspace` 最终递归删除目录。当前开发 workflow 没有 before_remove 保全钩子，
控制器还会忽略 before_remove 的失败。仅在临时 workspace 写摘要清单不能保证原件留存。
这证明存在保全缺口；现存日志未给出 GH-25 删除的逐文件记录，不能据此断言此次丢失的
唯一原因。宿主 Docker fixture 清理脚本不删除 workspace，二者不能混为一谈。

下一项先处理开发工作区的证据保全：交接/清理前把声明的必需证据复制到独立目录，
校验原清单摘要并持久化位置；任何必需文件缺失或校验失败时保留原工作区并显示原因，
测试中断重试及失败不得继续删除。不能靠一个失败会被忽略的 hook 宣称已修复。
当前产品就绪异常另行只读诊断，恢复时沿用既有 owner、预算及外部事实。
随后再按 [M1](../../daily-use-v1.md) 推进草稿导入与父子需求模型。

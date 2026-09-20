# 开发工具收尾与下一项工作（2026-09-20）

后续结果：[A13 证据恢复](../gh25/evidence-recovery.md) 已恢复 8/12 项原清单文件。
下文“12 个原路径不可读”保留为当时快照，以新恢复清单中的可读位置与剩余四项为准。
本批 PR #54 随后双 Gate 成功并合并为 `4d73c7d`；下文交付边界是提交时记录。

产品基线为 `f4e0487e55c42e75292b9c4e4a03452104a6e9da`，已包含 GH-22～25。
本次将开发控制器恢复桥接、一次性夹具清理、安装器及部署说明收进独立分支。
补充修复宿主恢复 API 默认跟随 HTTP 重定向的问题：拒绝重定向，防止转发令牌或重放恢复。
真实本机 HTTP 测试覆盖 GET/POST、301/302/303/307/308、同源和跨源目的地。

## 本次验证与交付边界

- `python3 -m unittest discover -s tools/symphony -p 'test_*.py' -v`：28 项通过。
- `python3 tools/gate.py config check`、秘密扫描、架构检查、`git diff --check`：通过。
- 原部署记录中的 26 项 Python 测试及 357 项 Elixir 测试属于此前部署，本次没有重跑 Elixir。
- 新增重定向修复已于 04:25 UTC 安装到运行中的 operator；未重启开发控制器或修改产品占用/预算。
- 本分支尚未取得自身精确提交的正式双 Gate；历史产品检查不能替代本次交付检查。

部署源码提交为 `0231d6bbff9158c28971d6fbf8bd09bc9653ec2a`。对已安装模块执行的 9 项
恢复/传输测试通过，恢复 API 未鉴权返回 403、带鉴权的不存在 Issue 返回 404。
产品 health/database 正常，开发控制器和两个 timer active，operator 最近一次执行成功。
部署前后 token、grants、workflow、handoff journal 摘要一致；备份、测试日志和
`verification.json` 位于宿主
`~/.local/share/codexsymphony/symphony/operator-deployment/20260920T042432Z-closeout/`。
已部署 `operator_bridge.py` SHA-256：
`cf9d4a66de0d2eae3a917eb9f74984cf3ca8a945a80be2ecf81ecb47c5c623f1`。

## Phase 0a 证据核对

| 范围 | 已核实 | 剩余事项 |
|---|---|---|
| A01 | [GH-24](../gh24/README.md) 有真实浏览器、Run、候选、独立验证及产品 PR 记录 | 保持历史运行身份；不以重新运行覆盖原证据 |
| A02–A12 | [现有映射](../../single-repository-acceptance.md) 指向 Runtime、恢复、预算、存储与页面回归；GH-25 记录 101 项 Rust、20 项前端单测、16 项 E2E | 本次核对索引及历史结果，没有再次运行产品全套测试；发布前逐项引用适用证据 |
| A13 | [GH-25](../gh25/README.md) 记录首仓占用时零执行、精确合并后领取第二仓、独立验证和 PR | 12 个原始证据路径目前不可读，需要定位保留副本并校验摘要 |
| 正式双 Gate | [PR #52](https://github.com/musutrade/CodexSymphony/pull/52) 与 [PR #53](https://github.com/musutrade/CodexSymphony/pull/53) 均已合并，Harness-Gate / Trusted Harness-Gate 均 SUCCESS | 这些检查绑定各自 PR head，不将状态移标到新提交 |

PR #52 检查绑定 `e21363d9d2957548380d09b8726c097d2817fbfa`，运行
[35478012419](https://github.com/musutrade/CodexSymphony/actions/runs/35478012419)；
PR #53 绑定 `4ee49561f05a4e4f048457ead34ffc87187fa207`，运行
[35484273886](https://github.com/musutrade/CodexSymphony/actions/runs/35484273886)。
原文档“精确提交双 Gate 待验收”是当时快照，上述查询补充其后续结果。

[逐文件复核](evidence-audit.json)：GH-24 的 82 项全部 SHA-256 匹配，25 项仍在原路径，
其余 57 项位于宿主 `a01-gh24/retained-artifacts/`。GH-25 的 12 项均未在原路径找到；
已搜索宿主 artifacts/evidence、产品部署、workspaces、Symphony 目录及 `/data` 下的相关归档目录，
尚未找到可直接匹配的副本。此结论不证明原件永久丢失，也不推翻历史 PASS；但不能宣称完整可追溯发布。
主规格发布清单继续保持未勾选。

## 下一项：A13 原件定位与发布索引收尾

先定位 GH-25 保留清单的 12 份原件或归档成员，逐项对比既有 SHA-256；更新可读位置与
保留状态，保留原路径和原摘要。找不到的条目明确标记不可用，评估它对应的验收条件是否
已有独立保留证据；确需补验时建立新的运行身份，不能改写旧摘要或把当前响应冒充旧原件。
验收结果是 A01–A13 各项均有适用证据、精确提交检查与可读保留位置，未满足项明确列出。
此项不需要重新开发多仓路由，也不授权合并测试仓 PR、释放产品 owner 或刷新预算。

随后进入已确认的 [M1](../../daily-use-v1.md)：首项细化为“导入草稿与父子需求持久化”。
支持 Markdown/结构化输入形成 Draft 并记录来源；保存父目标、带 ID 的 AC，以及子项
kind、顺序、依赖、仓库、独立 AC 和验证计划。导入不产生 Ready、Run 或模型执行，
旧单需求保持兼容，重启可读回；校验非法 kind、悬空依赖与版本冲突。
自然语言生成、覆盖映射及原子整组评审、依赖领取按后续 M1 子项逐步接入；不能把上述
草稿基础宣称为 B01 整条自动交付通过。M2/M3 与五条有限试用记录沿用既有契约。

# M2 收尾与 M3 开发基线

核对日期：2026-09-21。本文记录开发起点，不改变 [日常 V1 契约](daily-use-v1.md) 或业务验收条件。

## 2026-09-23 排期增补

最新核对主干为 `4163e4f`：包含 #88 的原项关联修复（PR #100）及存储故障范围/Codex 版本修正（PR #101，锁定版本 0.156.1）。下文 M2 版本与工具锁记录是历史快照。#89/#90 暂停，#91 未启用；代码合入不代表 M3 完整验收或生产部署已完成。

下一轮[改造需求](extension-requirements.md)与[协议](extension-protocol.md)仅整理为文档；执行顺序以改造需求第 4 节为准，所有新 Issue 保持待排期。

## 版本与交付

M2 六项 Issue 均已关闭，对应 PR 均已合并。M3 从包含下表全部合并的最新 main 开始；本次固定起点为 `5c7934f6d86ba16f552b18c2bf90a19748f1a648`。M1 阶段终点为 `70d4f9c04c57c4549fcc07b0846a237914af7875`。

| Issue | 交付内容 | PR | 合并提交 |
|---|---|---|---|
| #71 | 账号、持久会话、业务 API 认证 | [#77](https://github.com/musutrade/CodexSymphony/pull/77) | `c5244f08c8d5ce3b82eacc17865be58c327aa5ae` |
| #72 | HTTPS、源站隔离、执行凭据边界 | [#78](https://github.com/musutrade/CodexSymphony/pull/78) | `49923294211686fc7746452d674e2d2dcec610d5` |
| #73 | 手机流程、隔夜回答恢复 | [#79](https://github.com/musutrade/CodexSymphony/pull/79) | `2f9dcb29bb5a6398766ca2aec86d1fce1a30d462` |
| #74 | Bark 通知、持久待办去重 | [#80](https://github.com/musutrade/CodexSymphony/pull/80) | `56b58b752a7c0830d1c99d6320e4d5443fca2628` |
| #75 | 加密备份、升级保护、隔离恢复 | [#81](https://github.com/musutrade/CodexSymphony/pull/81) | `86f19bacad6c36d025719a61ab9cc46c55e01c77` |
| #76 | 同版本集成、上线清单 | [#82](https://github.com/musutrade/CodexSymphony/pull/82) | `5c7934f6d86ba16f552b18c2bf90a19748f1a648` |

## 验证身份

GH-76 的受控集成产品源码基于 `86f19bacad6c36d025719a61ab9cc46c55e01c77`；该 PR 修改验收工具和文档，原始测试、产物摘要和环境身份见 [GH-76 记录](quality/gh76/README.md)、[验收索引](quality/gh76/acceptance.json)及[迁移与环境](quality/gh76/environment.json)。

GitHub 回读确认 PR #82 最终 head 为 `59b06be08d36fce51bcc1268a69ae293a83b8ed3`，以下检查均为 SUCCESS：

- [Harness-Gate](https://github.com/musutrade/CodexSymphony/actions/runs/35589366364/job/106300018481)：2026-09-21 10:50:05 UTC 完成。
- [Trusted Harness-Gate](https://github.com/musutrade/CodexSymphony/actions/runs/35589366364)：2026-09-21 10:49:51 UTC 完成。

PR 于 10:50:17 UTC 合并。上述结果绑定 PR head，不将其改记为 merge SHA 上重新运行的检查，也不代表生产业务验收。后续代码变更仍需自身精确版本的门禁。

迁移终点为 `0024_action_notifications.sql`，GH-76 受控环境记录 24 条成功迁移及 checksum；生产数据库版本需部署时实际核对。工具锁定沿用根目录 `codex-version.lock`（0.154.0）与 `harness-gate-version.lock`（Core 0.4.5 / Rust collector rc.6），不更新策略或阈值。

## 当前完成边界

| 层次 | 状态与依据 |
|---|---|
| M1/M2 代码交付 | 已合并，M1 使用说明见 [交付索引](m1-delivery.md) |
| M2 受控集成 | passed；按 [M2 矩阵](m2-delivery.md)限定的 AAuth01–06、B02/B03、B07/B08 的 M2 部分及恢复场景 |
| M2 真实部署 | blocked；实际公网/代理、实体双设备与 Bark、生产执行权限、加密离机目标和恢复切换仍需现场证据 |
| 日常 V1 完整交付 | 未完成；M3 的 B01、B04–06 及 B08 CI/自动合并部分仍待实现与验收 |
| 历史 A13 原件 | 四项 unresolved，沿用 [原件恢复索引](quality/gh25/evidence-recovery.md)，新证据不替换旧原件 |

生产前置配置、操作步骤及回退条件集中维护于 [M2 上线清单](m2-delivery.md)，本文不重复一份部署操作流程。

## M3 起点

M3 依照日常 V1 第 7、8 节推进 CI 有限恢复/修复、自动合并、精确版本的子项业务验收、validation_only 执行、父项集成与原授权内关联修复。继续保持全局串行、项与父组累计预算、暂停/取消/保存语义和三个真相的边界；不得把 PR merged、CI success 或 Gate PASS 直接写成业务 Done。

开发时从最新 main 建独立分支并确认包含本基线；前置项更新后重新核对版本和证据。受控测试使用独立资源，真实上线状态按现场记录更新。本次文档整理未部署服务、迁移生产库或重跑产品集成。

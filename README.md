# CodexSymphony

个人 AI 开发编排系统：评审需求后由 Agent 编码，平台负责验证、PR、CI 和交接。长期目标是正常路径自动到合并与业务验收完成。

当前为 **设计规格 + spike 阶段，尚无平台主体代码**。当前实施范围是 Phase 0a：localhost 上第一条需求到 PR，随后加入多仓登记，始终严格全局串行。

## 从这里开始

| 文档 | 效力 |
|---|---|
| [综合方案 V10](Personal_AI_Software_Factory_综合方案.md) | 当前实施契约；第 23 章为队列，第 24 章为验收索引 |
| [架构边界](docs/architecture-boundaries.md) | 三个真相分离，Harness-Gate 只提供验证证据 |
| [演进目录](docs/roadmap-specs/README.md) | 后期候选及启用条件，不构成当前开发/验收要求 |
| [本次范围收缩记录](docs/scope-reduction-2026-09-14.md) | 变更理由与迁移映射，不另定义行为 |
| [运行复盘](docs/symphony-harness-gate-retrospective-2026-09-13.md) | 历史事故依据，不覆盖当前规格 |

## 当前选择

- 一个 Rust 控制面 + PostgreSQL + 最小 Angular Web，60 秒只读 GitHub 轮询。
- GitHub App；Agent 只通过受控工具提交/声明，平台经持久化 outbox 发布。
- 严格全局顺序覆盖编码、验证、交接、CI 等待和阻塞；具体释放条件见综合方案第 6 章。
- 部署期强制静态网络白名单；需求联网声明只是评审意图，不承诺逐任务网络隔离。
- 保留启动恢复闸门、工作保全、精确验证身份、一次代码修复、累计预算和磁盘保护。
- 手机、执行隔离、Harness-Gate 接入及自动合并随后分期；通用租约/诊断/资源/缓存框架按实际需要评估。

## 已有实验

[S1](spikes/s1/README.md)验证动态工具与工作区写边界；[S2](spikes/s2/README.md)及[补测](spikes/s2/README_S2b.md)验证 App、PR、Checks 与 SHA 守卫；[S3](spikes/s3/README.md)验证 Access；[S4](spikes/s4/README.md)验证 Gate 配置身份；[S5](spikes/s5/README.md)验证全局网络配置。
实验结论绑定当时版本，不替代当前部署和实现验收。尤其 workspace-write 不限制同 UID 读取，0a 只接管本人可信仓库。

参考：[OpenAI Symphony](https://github.com/openai/symphony)、[Harness-Gate](https://github.com/musutrade/Harness-Gate)。

# 手机访问与执行加固候选

> Deferred: 0b / 0c · 非当前规范

## 0b：手机与一个通知渠道

可沿用 S3 的 Cloudflare Tunnel + Access 验证结果，Axum 验 JWT issuer/audience/subject/有效期，限制源站只能经 Tunnel 到达。
计划覆盖会话撤销、CSRF、通知失败重试、手机回答待办及断线重连；具体范围进入阶段时确认。
基本表单/键盘可访问性已属 0a，不推迟；响应式手机布局随远程访问交付。
暗色主题、设计 token、视觉回归、SSE、多通知渠道和完整 PWA 没有当前排期，按实际问题独立评估。

## 0c：信任边界

候选实现为独立 UID/容器 executor、执行组整体终止、模型认证和工具读取视图分离；S1 的同 UID 可读凭证问题是此项依据。
接管不可信代码之前先验证隔离效果，不以 workspace-write 冒充读隔离。
备份候选包含数据库与恢复/验证资料、升级前备份、禁用外部写入的隔离恢复演练。

Harness-Gate 可替换 custom provider，沿用 S4 的源码身份/配置身份比对结果，使用实际固定二进制和受信入口。
集成不重写 CRAP、覆盖率、baseline/ratchet 等语义；缺失/不可信证据不视作通过。
本次收缩没有降低既有项目质量阈值。首次接入前依据届时固定版本重新确认命令与证据字段，不机械沿用历史 0.3.7。

完整双租约、dispatch_epoch、worktree_leases 和通用资源管理不再作为 0c 的当然前置条件。
单进程部署继续沿用主规格最小恢复闸门；只有出现多个独立执行者后再评估 fencing 方案。

## 网络授权候选

0a 的部署静态白名单语义见主规格第 16 章。逐任务声明目前没有逐任务强制隔离承诺。
未来若需要更窄授权，可评估独立执行环境/代理策略或经过实测的 session 能力；不假设上游已提供。
如果仍依赖全局 requirements 文件，需要评估所有受影响进程、变更授权、恢复及热更新语义，不能静默替换当前部署。

## 诊断与运维候选

只有固定探针仍无法解决的故障造成明显人工负担时，再考虑独立只读诊断 Agent。
候选报告区分 confirmed/suspected/unknown；模型假设不直接授权恢复、执行 shell 或修改预算。
通用 DiagnosticBundle/DiagnosisReport/DiagnosticAttempt/action catalog 不因进入 0c 自动启用。

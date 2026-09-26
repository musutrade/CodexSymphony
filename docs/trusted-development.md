# 可信开发环境：2026-09-16 执行边界变更

本次按用户确认采用 Symphony 的可信开发环境方式，替代多层命令沙箱、静态网络白名单和逐测试宿主验收入口。只接管本人可信代码，不提供恶意代码隔离或多租户承诺。

当前开发顺序以根目录 [AGENTS.md：Development procedure](../AGENTS.md#development-procedure-single-source-of-truth) 为唯一入口；本文定义执行边界，下面的命令说明不覆盖测量前置条件或最终 Gate 要求。

| 职责 | 当前边界 |
| --- | --- |
| Agent 开发 | 准备好的环境内正常运行 Git、构建、服务和测试；Codex approval_policy=never、danger-full-access |
| 项目预检 | 相同身份、cwd、工具和环境；检查依赖能力、可写路径、服务连通性与容量；不消耗编码模型 |
| 本地固定候选检查 | 普通受监督进程；前后核对 SHA/tree/入口，保留退出码、日志、超时和输出限制；不声称 namespace 只读保护 |
| GitHub 操作 | 独立授权接口持有凭据，Agent 只提交获准请求；不将凭据注入开发环境 |
| 正式验收与签名 | 独立服务获取指定提交，使用获授权配置完成双 Gate；签名密钥不进入开发环境 |

开发测试输出不是正式签名结果。本地测试可以运行，不以缺少受审查二进制 manifest、专用入口或宿主回执阻塞任务。正式验证的精确提交、真实采集、CRAP ≤10、覆盖率 ≥80% 继续生效。

部署可以使用专用账号、虚拟机或单个预配置环境边界。当前主机复用已有环境级文件系统边界，隐藏控制器/GitHub/Gate 状态；内部不再叠加 Codex 命令沙箱或测试沙箱。这是部署选择，不是要求每个项目开发执行适配器。

原 `execution_readiness`、`product_preparation_acceptance`、`runtime_product_acceptance`、`backend_tests` 等逐测试宿主入口退役。真实 Runtime 用 `cargo test --workspace --locked --test runtime_real` 验证；完整后端统一运行 `python3 /opt/symphony-env/workspace_tests.py`，由它核对锁定 Codex 版本、重建一次性 test 数据库、检查资源限额并串行执行 `cargo test --workspace --locked`。通用 `run.py` 仍只注入数据库配置并直接执行命令。模型协议测试使用本地脚本化 provider，不产生外部模型调用。

项目接入需列出实际依赖和测试命令，开工前核对环境就绪条件；测试执行遵循 AGENTS.md 的阶段顺序。不要因新增测试而新增宿主执行协议。普通环境失败与代码失败继续区分，历史重试、预算、工作区和失败证据不清零。旧文档和已关闭 Issue 的验收结果保留历史事实，不再定义当前执行要求。

变更同时覆盖 Rust Runtime/预检/本地验证、开发控制器安装配置、环境模板、当前规格和 #12–#25/#27 的执行约定。GH-21 的未完成业务实现需在新基线继续，不能把环境变更当作业务任务完成。

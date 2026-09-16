# 可信开发环境预检

当前执行边界见 [可信开发环境](trusted-development.md)。预检只识别实际依赖与环境故障，不建立另一套执行或网络授权平台。

`preparation_service::prepare` 核对 Launch、Broker/Workspace 和存储，保存尝试后调用 `tools/preparation/app_server.py`。adapter 在与 Runtime 相同的环境和 cwd 中启动锁定 app-server，只发送 initialize、command/exec，不启动编码 turn。

部署配置包含 launcher、uid、deployment_identity、dependencies、writable_paths、可选 network_urls 和 probe_path。probe_path 缺省为 `tools/preparation/environment_probe.py`。不再接受静态白名单身份或 allow/deny/bypass 证明作为执行前提。

实际检查工具版本/能力、文件写入/read/fsync、构建和临时路径的容量，以及指定服务 URL 的连通性。网络声明是依赖说明，不是逐任务防火墙。证据保存执行身份、环境版本、原始失败与步骤输出，身份错配不能沿用旧成功。

准备阶段最多初次加两次重试，退避 30/120 秒，期限 600 秒；与代码修复次数分离。未知执行先对账，暂停/取消不因探测通过解除。ENOSPC 或持久化故障阻止新工作和外部写入，保留原件。保留现有 preparation_record/history 和 storage_guard，不重建状态机。

普通 Cargo 测试直接验证 Rust 持久化、失败分类和 Runtime 集成。无需宿主安装 reviewed-preparation/reviewed-runtime 或逐源码 manifest；正式精确提交 Gate 仍独立执行和签名。

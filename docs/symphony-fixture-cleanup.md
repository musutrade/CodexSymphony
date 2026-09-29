# Symphony 开发环境回收

> **旧开发控制器的按 Issue 环境回收说明。** 当前开发 Gate 使用固定数据库槽位；旧控制器仍停止派发，不能据本文重新安装按 Issue 创建资源的流程。历史资源清理仍须核对实际归属和已安装批准。当前策略统一见 [存储与证据保留策略入口](quality/evidence-retention.md)。

每个开发 Issue 的 PostgreSQL test/dev 容器由
`tools/symphony/provision_issue_environment.py` 创建。任务工作区删除不会自动删除
这些宿主 Docker 资源。`cleanup_issue_environments.py` 补齐这个生命周期。

运行 `python3 tools/install_symphony_cleanup.py` 安装独立的用户级 systemd timer。
开发环境安装器也会安装它。timer 每五分钟检查一次，不需要重启 Symphony，
不修改模型设置或正在运行的工作流。

默认预览：`python3 tools/symphony/cleanup_issue_environments.py`。
显式执行：`python3 tools/symphony/cleanup_issue_environments.py --apply`。

回收必须满足宿主 handoff journal 中该 Issue 为 `done`，且记录了本仓库的合并提交。
脚本同时核验 Docker 名称、`codexsymphony.fixture` 标签、挂载和网络消费者，
执行前重新读取状态。阻塞、运行、等待 CI、缺少合并证据或归属冲突均不能触发删除。
多个回收进程通过文件锁互斥；中断后可以重新扫描并清理余下资源。

执行顺序是禁用该 Issue 的数据库 broker，保存计划及最近数据库日志，停止和删除
专属容器，再删除专属开发数据卷及网络。数据卷内容作为一次性测试数据被删除。
审计记录保留在 `~/.local/share/codexsymphony/symphony/fixture-cleanup/`。
共享镜像、其他项目、产品 A01/A13 数据库、凭据、预算及产品恢复资料不属于回收范围。
宿主环境辅助程序目录保留，避免破坏其他运行说明或诊断引用。

当时门禁证据由 `codexsymphony-archive.timer` 单独管理；这是历史接线，不定义当前保留策略。本脚本不以开发任务完成为理由删除产品验收证据。

验证：`python3 -m unittest discover -s tools/symphony -p 'test_fixture_cleanup.py' -v`。

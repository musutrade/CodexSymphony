# 项目环境准备

当前方式见 [可信开发环境](trusted-development.md)。`before_run` 调用安装后的 `provision_issue_environment.py`，为项目准备测试/开发 PostgreSQL、缓存、浏览器和参考源码。相同任务重用已存在的资源，不重置工作区。

`python3 tools/install_symphony_development.py` 安装环境启动器、通用项目配置和工作流；不会启动调度。`trusted_environment.py` 只提供当前主机的环境边界，不挂载 GitHub 凭据或 Gate 签名状态。Codex 普通命令不再套第二层沙箱；本地 Git 可写。

`/opt/symphony-env/run.py` 注入项目数据库 URL 并直接执行命令，不做 SOCKS 转发和网络政策判定。`dbctl.py` 仅管理固定 test/dev 数据库。参考源码仍只读；它不是项目工作区。

安装验收运行 `check_environment.py` 及项目实际 Cargo/浏览器测试。删除旧的逐测试宿主入口，不再安装二进制 manifest 或要求源代码修改后人工重新批准测试。

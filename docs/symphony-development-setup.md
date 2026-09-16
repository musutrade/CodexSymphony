# Symphony 开发控制器

采用 [可信开发环境](trusted-development.md) 的执行方式。调度、预算、Issue/PR 交接继续由独立 Elixir Symphony 服务负责；Rust 产品不复制开发控制器的交接文件协议。

1. 准备锁定 Codex、Rust、Node、数据库与浏览器资源。
2. GitHub 凭据放在控制器的独立配置中，Gate 签名放在独立验收服务；不复制到项目环境。
3. 运行 `python3 tools/install_symphony_development.py` 安装工作流及环境模板。
4. 在准备好的环境中运行 `python3 tools/symphony/check_environment.py` 和项目实际测试。
5. 验证精确提交的双 Gate 后启动控制器。启动/暂停不重置预算和历史事实。

工作流固定 gpt-6-astra、medium，approval_policy=never、danger-full-access。普通 Git 和测试可执行，不要求强制网络白名单或测试专用宿主接口。项目数据库配置可以用 `python3 /opt/symphony-env/run.py COMMAND ...` 注入。

已关闭环境任务及旧沙箱记录仅供追溯，不覆盖当前边界；新项目应验证自己的真实构建和测试能力。

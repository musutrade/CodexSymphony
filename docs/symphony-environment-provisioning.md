# Symphony 按任务配置执行环境

`before_run` 在模型启动前调用宿主安装的 `provision_issue_environment.py`。
它接受分配工作区根目录下的任意 `GH-数字` 目录，不依赖具体 Issue 编号区间。

- 每个任务有独立的 PostgreSQL 16 临时库、合成持久库、内部 Docker 网络及 npm 缓存。
- 新网络从 `172.30.0.0/16` 选择不与已有 Docker 网络重叠的 `/24`；重试复用已有网络和容器。
- 沙箱仅放行该任务两个数据库地址；源码、浏览器资源只读，Docker socket 和宿主凭据不开放。
- 模型读取 `.agent-env/README.md`，使用 `/opt/symphony-env/run.py` 在命令沙箱内启动数据库转发。
- `/opt/symphony-env/e2e.py` 启动真实 API，并设置测试 Origin；npm 按锁文件离线安装。

## 安装与保留

`python3 tools/install_symphony_development.py` 安装仓库中的启动钩子、通用沙箱入口、
配置脚本和客户端模板。已有安装保留操作者的 `required_labels`，首次安装仍要求环境验收标签。
安装不重启执行中的任务；工作区、数据库卷、handoff journal 和历史用量不会被重置。

宿主的 `~/.local/share/codexsymphony/symphony/environment-template/` 保存已准备的离线资源：
固定版本 arc-admin 源码及来源记录、固定 PostgreSQL 镜像的 psql/动态库，以及 npm 缓存种子。
这些资源独立于 GH-12 工作区，重装代码时保留。源码模板位于 `tools/symphony/environment/`。
新宿主必须先准备这些资源；缺失时安装器在替换现有 workflow 前明确失败。
这套安装流程不把开发机离线缓存或二进制文件提交到 Git。

## 验证与边界

```sh
python3 tools/symphony/test_environment_installation.py
python3 /opt/symphony-env/run.py cargo test --workspace --locked
python3 /opt/symphony-env/run.py python3 /opt/symphony-env/e2e.py
```

环境配置是可重复的，不会在重试时清空数据库。显式调用 `dbctl.py recreate test` 才会丢弃临时库；
`recreate dev` 保留命名卷。销毁任务环境及其持久卷是独立的宿主维护操作。

这解决了新任务漏配已有环境能力的问题。新依赖不在离线缓存中、Docker 故障、地址池耗尽或新增外部
能力仍会明确失败，需要补充环境；它不保证所有业务任务永不阻塞，也不替代新增代码的验收。

# 一致的开发环境与发布前验证

`environment.lock.json` 是环境版本和资源策略的唯一人工维护入口：Rust、
Node/npm、Python、Codex、Gate/collector、工具二进制摘要、PostgreSQL 镜像摘要、
test/dev 内存、swap、CPU 和测试调度参数。test 和 dev 的角色可以不同，同一角色
在开发与 Gate 中必须相同。内存与 memory-swap 使用 Docker 的字节语义；后者是
内存加 swap 的总上限。

当前单宿主部署直接复用已安装工具；清单只选择和检查工具，不下载、不安装第二套。
人工开发使用下方 `shell` 入口，Symphony 与 Gate 使用同一模块选择 PATH 和测试参数。
开发命名空间只挂载宿主工具，独立的是工作区、临时文件和数据库数据。仓库的依赖
若不满足就停止并报告差异，不由 Agent 自动升级宿主。其他仓库可有不同需求；本次
部署仅管理 CodexSymphony，不把本仓库的数据库和工具版本强加给其他仓库。

`.node-version`、`rust-toolchain.toml`、Codex/Gate lock、Compose 的镜像和资源项、
Gate flow 的镜像都是工具兼容投影。升级时执行：

```sh
python3 tools/environment_contract.py sync
eval "$(python3 tools/environment_contract.py shell)"
python3 tools/environment_contract.py check
```

`check` 检查实际选中的工具版本、二进制摘要和测试环境变量。安装版本携带同一
清单；工作区清单与宿主安装版本不同、安装脚本被修改、生成文件不同或数据库资源
不一致都会明确失败，不会默默换版本或调整正在使用的数据库。工具版本升级需
更新清单、对应二进制摘要并重新安装经过验证的宿主发行。Node 依赖继续由
`package-lock.json` 固定，不复制第二套依赖版本表。

Symphony 的 before_run、实际开发命名空间、正式 Gate 都检查同一清单。每次 Gate
保留 `environment.json` 和各数据库的实际镜像/资源检查记录。启动验收使用
`python3 tools/symphony/check_environment.py --workspace <已准备的GH工作区>`，
直接比较宿主与开发命名空间指纹，无模型调用。

## 发布路径

1. 完成代码和证据文件；日常可使用普通测试快速迭代。
2. Agent 调用 `local_gate {"action":"start"}`。宿主启动独立 systemd 作业，执行
   正式 Gate 的同一个完整入口，签名凭据仍留在独立验证边界。
3. `local_gate {"action":"status"}` 返回 RUNNING/PASS/FAIL/STALE；失败结果带有
   有界诊断。停止源码修改，取得完整 PASS 后才发布。
4. 宿主 `github_api` 在创建/更新分支、创建/编辑 PR 前，从 GitHub 读取候选 commit
   的 tree，校验与本地凭据绑定的 tree、环境指纹、批准版本和完整报告一致。
   Git tree 包含新增、修改、删除、可执行位；本地索引不会被验证命令修改。
5. GitHub CI 独立校验发布后的精确提交，继续维持原覆盖率和 CRAP 门槛。

Agent 无法从自己的工作区生成宿主凭据。直接 contents API、force update、其他
分支、其他仓库、删除和替代发布接口默认拒绝；上传尚未引用的 blob/tree/commit
允许继续使用。没有凭据、验证失败、源码或环境变化、验证进程中断均不能发布。
工作流文本不是唯一约束：实际凭据持有方的发布入口实施拦截。

开发控制器通过只读 `/etc/codex/requirements.toml` 固定 `features.apps=false`，
并配置空 MCP 允许清单；普通配置和命令行不能重新打开 Apps。GitHub 写操作只走
控制器注入的 `github_api`，本地 Git 仍可正常使用。这是可信开发部署的能力配置，
不声称能隔离主动窃取模型登录凭据的恶意程序。
配置语义参考 [OpenAI 配置文档](https://developers.openai.com/codex/config-reference/)。

凭据和日志保留在 `~/.local/share/codexsymphony/publication/GH-N/`，不加入源码。
失败日志只通过工具返回尾部；完整采集证据仍在原 Gate run 目录。

## 安装与维护

仓库包含 `tools/symphony/controller-publication.patch`，与已有开发控制器补丁并列；
它只改独立 Elixir 开发控制器的工具边界，不修改 Rust 产品的交付状态机。
控制器执行 `make all`、构建后记录 binary/patch 摘要；开发环境安装器校验该记录，
防止新工作流被交给一个不支持发布拦截的旧控制器。

先完成本地完整验收并安装 Gate，再安装 publication/development 环境。旧环境
资源不自动删除；升级资源需显式维护。安装不启动 Symphony，暂停状态应保持到
操作者明确恢复。仓库合并与部署是独立步骤；旧分支缺少新清单会明确阻断，不能
拿旧版本清单与新宿主混跑。

安装顺序为 Gate → remote Gate → publication/development。remote Gate 安装器核验
systemd 的有效 ExecStart，旧 drop-in 覆盖会立即报错。开发服务启动及每次 before_run
执行 `check_deployment.py`，检查有效服务命令、运行中的 remote Gate 命令、控制器
和发布入口摘要，以及本地与 CI 使用的 Gate approval 是否一致。仅写入新 unit 文件
不算部署完成；实际进程仍使用旧配置时阻止接单。历史覆盖配置应归档并恢复单一 unit
入口，磁盘和挂载保护等无关配置保留。

## 编译缓存

宿主执行 `python3 tools/install_sccache.py` 安装清单锁定的 sccache；下载包和二进制
均校验 SHA-256。继续通过上面的 `environment_contract.py shell` 进入开发环境，
无需修改个人 `~/.cargo/config.toml`。Symphony 和 Gate 显式使用同一个宿主二进制、
`RUSTC_WRAPPER` 与缓存配置，Rust 增量编译关闭；版本和配置纳入环境指纹。

缓存上限由清单中的 `SCCACHE_CACHE_SIZE` 控制。宿主使用持久开发缓存，Symphony
各工作区使用 `target/sccache`。Gate 从主干完整 PASS 发布的种子恢复独立副本，
PR 和开发进程不能改写公共种子。sccache 内容随现有编译缓存一起管理容量与 TTL；
不会复用旧测试结果或覆盖率计数。各隔离命令使用自己的 Unix socket，结束前保存
统计并停止缓存服务，避免连接到宿主进程或其他任务的缓存服务。

Gate 的 `sccache-stats.json` 保留各阶段命中、未命中和不可缓存请求；宿主在上述
开发 shell 内可执行 `sccache --show-stats`。涉及链接的编译无法缓存；已有 Cargo
依赖缓存命中时，sccache 的额外收益可能有限。覆盖率构建保留真实源码路径，
不通过忽略路径或源码差异制造命中。比较耗时需同时查看 Cargo 编译和完整 Gate。

# 一致的开发环境与发布前验证

`environment.lock.json` 是环境版本和资源策略的唯一人工维护入口：Rust、
Node/npm、Python、Codex、Gate/collector、工具二进制摘要、PostgreSQL 镜像摘要、
test/dev 内存、swap、CPU 和测试调度参数。test 和 dev 的角色可以不同，同一角色
在开发与 Gate 中必须相同。内存与 memory-swap 使用 Docker 的字节语义；后者是
内存加 swap 的总上限。

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

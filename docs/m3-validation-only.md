# M3-4：精确版本纯验证

本项是 GH-87 的增量能力，不表示 B01–B08 或生产现场验收已全部完成。

整组评审中，`validation_only` 的 `integration` 显式绑定部署提供的受信验证配置
SHA-256、每仓登记策略版本、版本选择规则和修复范围。缺少这一字段的旧授权保持
等待，不自动升级。Draft、尚未确认的评审以及过期授权不能执行。

`fixed` 使用评审指定的完整 SHA；`completed_dependencies` 使用已完成前置项的
合并版本。领取读取实际完成事实，并检查同仓包含关系；跨仓分别绑定 SHA/tree 和
产物引用，不做跨仓祖先推断。部署需在既有 Broker 对象库准备这些精确版本。
配置由 Runtime 仓库路由提供，HTTP 不能指定宿主可执行路径。

每次验证持久化独立 `integration_validation` 身份、授权/revision、输入摘要、完整
版本组合、受信配置/命令/入口身份、工作区和启动意图。它使用全局 Requirement owner
和已有 subreaper，不建立 AgentRun、提交或 PR。主仓工作目录及每仓检出保存在
`workspaces/runs/<原验证身份>-repo-<仓库ID>`；固定集成工具可从主仓目录定位同一身份
下的兄弟仓库。恢复保留原检出，并给新的验证调用分配新身份和输出目录。

所有步骤使用既有验证执行器的超时、输出上限及净环境。开始和执行期间核对暂停、
取消、仓库权限、存储状态和授权身份。监督进程身份及 quiescent 回执匹配后才读取
结果；源码、受保护入口、调用身份、步骤输出或版本集变化均不能借用旧结果。
未知启动或丢失监督进程保持占用，不能并行重跑。

失败结果、日志和旧调用不会被后续通过覆盖。已知停止或可识别的基础设施故障按
M3-2 的 30/120 秒退避、最多两次自动重试和阶段 deadline 恢复，不消耗编码修复次数。
未知结果先对账；超限保留阻塞原因。代码修复目前等待 M3-5，只保留当前项，不创建
依赖当前项完成的修复后继，也不重写已完成代码子项。

`group_acceptance` 只在所有必需子项完成、当前父 AC 覆盖有效、最终集成项的精确
版本验收通过时写入。界面依据当前授权和父/评审版本读取 Done，父项不领取 owner。
暂停/取消与完成采用同一协调器事务锁；取消只在验证进程确认静止后释放占用。

## 迁移与回退

`0028_integration_validation.sql` 增加验证账本、同项单个未静止调用的唯一索引和
父项验收表，不改历史 AgentRun、交付或完成事实。既有 group_review JSON 新增可选
`integration`，未提供的旧数据仍可读，且不会被自动授权执行。

回退前暂停队列，等待全部验证监督进程静止并保全数据库和原输出。旧二进制不能
安全处理新验证 owner 或新种类完成事实，不应在活动验证期间直接切回。优先保留
新表、停用新授权并回退部署配置；数据库降级需离线恢复迁移前的一致性备份，不能
删除活动账本或将纯验证完成事实改写成 PR 合并事实。本任务不执行生产部署或回退。

## 可复跑验证

在 `.agent-env/README.md` 所述一次性环境：

```sh
python3 /opt/symphony-env/run.py cargo test --locked --test integration_validation
python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/gate.py config check
```

前端沿用 `npm run lint`、`npm test -- --watch=false`、`npm run build` 和环境提供的
E2E helper。集成测试实际运行 PostgreSQL、Git 检出、受信命令及 subreaper，使用合成
授权和一次性本地仓库；不将其描述为真实 GitHub 合并、生产权限隔离或公网验收。
精确提交双 Gate 由独立宿主在 PR 发布后运行，工作区测量和测试不签发受信结论。

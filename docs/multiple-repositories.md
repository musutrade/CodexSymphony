# Phase 0a 多仓登记与串行路由

GH-25 的多仓入口为需求工作台中的仓库列表、目标仓库选择和“登记另一个仓库”。
登记保存策略，不证明 Runtime 或 GitHub 就绪；列表单独显示 GitHub 能力是否新鲜且匹配策略。
缺配置、权限或检查来源时保留队首，不启动模型，也不扫描后面的仓库寻找可执行项。

`/api/multi/repository` 与 `/api/multi/requirements` 系列提供多仓接口。
登记、Draft 创建/编辑的 `repository_id` 是平台登记 ID，区别于 GitHub 数字 ID；
省略时兼容登记 1。Ready 将登记 ID、GitHub 身份及策略版本一起冻结。
修改 Draft 的仓库选择必须保存后重新评审。已登记的远端和 GitHub 数字 ID 不可替换。
原 `/api/repository`、`/api/requirements` 的闭合响应格式保持兼容；新客户端使用多仓接口。

所有仓库仍只有一个 `execution_control` 占用者和一个 Runtime worker。
当前需求在 Submitted、CI 等待、暂停或人工阻塞中时，路由始终选择该需求；
只有满足现有合并事实/显式取消收尾条件才能释放。释放不写业务 Done。
从选择配置到领取期间，事务再次核对所选需求 ID/修订号；撤回、改仓或重新评审
不会让新队首继承上一仓的 baseline/launcher。

## 操作员部署

原单仓 `RUNTIME_CONFIG` 和 `GITHUB_APP_CONFIG` 继续可读。多仓部署需在原服务上
替换配置文件，保留原数据库、`EXECUTION_DIRECTORY`、`STORAGE_CONFIG`、首仓资料和预算。
不要启动第二个控制器或使用新数据库来绕开占用。迁移 0016 只扩展登记约束并为既有需求
补登记 ID 1；不清空任何 Run、交付、存储或预算表。

多仓 Runtime JSON 的顶层为 `repositories`，键是平台登记 ID。每项包含
`github_repository_id`、`remote`、`base_branch`、`version` 和 `runtime`；
`runtime` 的内容是原有单仓 Runtime JSON（settings、preparation_adapter、preparation、validation）。
运行前核对登记和冻结评审的完整身份/版本。每仓使用其自己的 baseline 和验证计划。
缺路由或身份不匹配时停止在队首，不能回退到首仓配置。

GitHub JSON 的顶层为 `app`（原单仓 GitHub JSON）和 `repositories`
（附加仓的 `{ "policy": ..., "probe_pr": ... }` 列表）。所有仓复用 `app` 指定的同一个
App 身份；installation 和 repository-scoped token 继续由既有 AppClient 按精确目标解析。
新增仓库登记完成后重启原服务加载其能力契约；秘密始终留在操作员部署中。

平台仍使用同一个本地 Git 对象库和全局存储预算。操作员将每仓授权 baseline bundle
导入该对象库中互不覆盖的 `refs/import/repositories/<github-id>/...` 引用；不要覆盖首仓引用。
每个 Run 的随机身份、分支和独立 worktree 仍由原 Git Broker 建立并核验。
Runtime 配置指向具体 baseline SHA，交付使用冻结评审中的 repo/base 和 Run 候选。
存储 attempt 保留 repo/requirement/revision/Run 身份，沿用共享容量、活动消费者保护及
归并时的精确身份校验，不建立第二套回收器。

## 验收边界

`multiple_repositories` 集成测试覆盖登记、冻结目标、缺能力零 Run、跨仓策略版本、
占用期间路由不切换和旧单仓配置拒绝第二仓。既有 execution、delivery、runtime、storage
回归继续覆盖释放条件、独立业务事实、全局预算和活动恢复资料保护。
浏览器测试覆盖新增登记/选择控件的桌面、手机、键盘与 axe。

真实 A13 已在原产品服务完成：首仓仍占用时第二仓排队且模型调用为零，操作员审核合并
首仓 PR 后产品正常释放，再自动执行原第二需求并创建私仓 PR #2。真实 Runtime、候选
验证、PR、CI 及共享存储身份见 [GH-25 记录](quality/gh25/README.md)。操作员 probe PR
不作为产品 A13。A01–A12 证据索引仍见单仓验收文档，完整 0a 通过须汇齐第 24 章证据
并完成精确提交验收，不由开发自测或单个 PR 的 Submitted 推导。
localhost 0a 内部里程碑不包含手机接续、大需求分解或自动 CI 修复合并。

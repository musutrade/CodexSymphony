# 固定任务模型选择（GH-106）

项目的 `model_selection` 是新评审的默认值。单需求在 `contract.model_selection`、
整组评审在 `review.items[].model_selection` 指定显式覆盖；覆盖优先于项目默认。
两处均使用相同结构：

```json
{
  "config": {
    "provider": "deployment-provider-id",
    "model": "deployment-model-id",
    "effort": "low"
  },
  "reason": "本子项已评审的选择理由"
}
```

以上是结构占位示例，不能直接当作可用模型。provider 是 app-server 的提供方配置 ID，
model 是该部署支持的实际 ID；产品名称不会被自动转成模型。effort 必须明确填写。
旧 `repository.model` 及没有新字段的历史任务保留兼容路径；新默认不会回填历史快照。

## 部署登记及授权

在已有 `RUNTIME_CONFIG` 对应仓库的 `runtime.settings.model_capabilities` 登记
`version`、`repositories` 和已有 `AgentCapability` 结构的 `agent`。
`agent.name` 为 `codex`，`agent.models` 是允许的完整 provider/model/effort 元组列表，
并声明 reliable_stop、resume、cancel、structured_events、usage_reporting 能力。
单仓库配置使用 `settings.model_capabilities`。配置不包含在需求输入内，Agent 工具不能改写它。
必须同时满足仓库路由、能力列表、仓库授权、插件作用域和原预算限制。

登记版本描述部署能力版本；仓库版本描述本次默认与策略。修改部署配置按既有服务配置
加载流程生效，不能把修改配置文件等同于已运行进程完成重载。仓库及插件撤权继续通过
现有持久控制面执行。未登记、越界、缺少可靠停止/取消或不支持的组合明确拒绝，
不会选用另一个模型。启动请求关闭提供方模型 fallback。

## 评审与执行

保存整组评审时，服务端为每项生成 `frozen_model`，其中包含实际选择、来源、理由、
仓库及能力登记版本；客户端提交的冻结结果不能替代服务端计算。确认时重核评审结果，
配置变化要求重新评审。授权快照包含各子项的预算和选择，一次整组确认即可排队。
单需求的 Ready 评审也将选择写入不可变 revision。后续改变走原变化评审和未启动项
授权流程，累计用量与预算身份不清零。

初始执行、继续、回答恢复及修复都从 execution revision 读取冻结选择。每次付费 turn
继续核对冻结的能力登记版本、能力和授权，并明确发送同一模型与 effort。thread/start 响应中的模型、提供方
和 effort 必须与冻结选择一致；不一致时记录实际响应并阻止 turn。项目 Hooks 的冻结
扩展配置使用同一选择。仅改变模型默认的仓库更新不会改变已有选择；其他路由/策略
变化仍按原边界阻断。

协议字段以锁定二进制生成的 schema 为准。公开协议说明见
[Codex App Server](https://learn.chatgpt.com/docs/app-server)。thread/start 提供身份确认，
turn/start 响应只提供 turn，因此不能把它描述为独立的模型身份回执。

## API、用量与迁移

沿用 `/api/multi/repository`、单需求 API、整组 review/authorize/queue-edit API。
评审读取返回各项 `frozen_model` 和预算；完整手机交互仍由 #89 承接。
`GET /api/requirements/{id}/models` 返回各 Run 的冻结配置、实际响应、匹配结果及已有
model_call 用量与预留。空值表示不可得；历史 Run 没有新增身份回执时实际身份为 null。
用量不完整仍保留原预算预留，cached input 是 input 的子集，不重复累加。

迁移 `0039_model_selection.sql` 只增加身份记录表，不重写历史选择、用量或账户。
接受新选择后不可直接降级到忽略冻结配置的旧服务；回退前停止并保全任务、数据库、
配置和实际副作用，以明确的新评审处理后续执行。不得删除新字段来让旧服务继续编码。

本项不自动升级模型、不迁移提供方会话、不实现完整手机/父组组合；#132、#89、#90
各自保留责任。受控协议服务、真实锁定 Runtime 加本地脚本提供方、线上付费模型须分别
标注。线上调用需要适用授权、原账户的累计额度和有限调用预留。

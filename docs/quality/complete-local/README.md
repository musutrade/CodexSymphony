# 完整本机门禁验收

本次完成 Rust 源码、TypeScript 和 HTTP JSON 合约三个独立 collector 的组合，
以仓库外受信宿主运行真实采集、签名请求和完整 Core 0.4.5 verify。
没有修改 Core，也没有放宽 CRAP ≤10 或覆盖率 ≥80% 的要求。

2026-09-15 的最终本机验收为 `run-ed89edc306d4`，安装的宿主版本为
`f3d999766a189f3c`。完整 ci profile 返回 PASS，证据完整，三个 producer 均成功。
此次输入是 `162b7734abdb3c514f50807f45febeeb9fcb1787` 加实际工作区修改；
精确文件清单与摘要在归档的 `source-inputs.json`，不能只按提交号复现。
本验收文档及负例脚本的断言修正是在完整运行后加入。

完整运行包括秘密扫描、架构检查、7 项构建／测试步骤和 20 条测量证据。
Rust 使用编译二进制与 profraw 重新导出覆盖率；Angular 对原始 TypeScript 插桩；
API 从真实运行服务取得 200 和数据库停止后的 503 响应。临时数据库为本次运行独占。
前端类型由当前 API 合约生成，来源摘要与实际类型均需匹配；初始基线固定在宿主侧。

宿主安装在 `~/.local/share/codexsymphony/gate-host/releases/`，入口是同目录上层的
`run --repository ~/CodexSymphony`。批准文件、基线和宿主实现位于工作区外。
采集阶段没有签名私钥，签名后私钥删除，验证阶段的源码和信任文件只读。
Linux namespace 验收确认测试无法读取宿主密钥目录、无法改写门禁策略。
源码输入、工具、捕获脚本、测量系列或政策发生不被批准的变化时会拒绝新运行。

`acceptance.json` 和归档记录精确 run 路径、源身份、插件版本／摘要及验收结果。
精简归档保留报告、源码／配置摘要、公开验签材料与标准化证据；原生二进制和完整
源码副本保留在本机 run 目录，独立重新导出需保留该目录及匹配的工具链。
归档不包含私钥。

## 防重放与负例

Core 0.4.5 完整 verify 的持久化 nonce 缺口已提交
[Harness-Gate #269](https://github.com/musutrade/Harness-Gate/issues/269)。
宿主侧 broker 将 ledger 保存在测试挂载之外，以独占创建记录消费；签名绑定的
collector 包装器必须先取得 claim。完整运行的三个首次 claim 成功，随后新进程
使用原签名请求、原 nonce 和空产物目录时，被重启后的宿主 broker 拒绝。
测试同时核对新增 broker 审计事件，避免仅由 Core 的诊断 ledger 拒绝造成误判。

五项真实负例均返回非零：重复 nonce、签名篡改、过期上下文身份、产物篡改、
缺少 HTTP 合约 producer。这里的“过期上下文”是源码提交身份不匹配，不是签名时钟过期测试。
首次负例脚本对错误文字的断言不符已修正；实际 Core 当时已拒绝旧上下文，
原失败记录仍留存在本机 `negative-checks-initial/`。随后完整五项检查通过，
标准化产物及请求均恢复原内容。负例脚本需在新运行的 15 分钟签名有效期内执行：

```sh
python3 tools/check_gate_host_negatives.py /absolute/path/to/fresh/run
```

旧验签材料用于审计，不能复制到新工作区充当长期授权。

这证明本机完整链路，不等同于远端 CI 已运行或 Symphony 已启用。
GitHub hosted 工作流已准备插件安装；远端宿主连接、必需检查／保护规则和
Symphony Issue→PR→CI→交接仍需后续验收。候选包不是上游正式签名发行。

# Rust 源码 CRAP 本地验收（2026-09-15）

用户已选择源码分支口径。独立 Harness-Gate collector `rust-source-risk`
0.1.0-rc.1 已实现并安装；Core 未修改，CRAP 上限仍为 **10**。
MIR 编译后复杂度保留作诊断，不能替代这个源码系列参与 CRAP 判定。

- 实现：`~/Harness-Gate-rust-source/tools/quality/rust-source-risk`，分支 `feat/rust-source-risk`，提交 `337c363`。
- 本机入口：`~/.local/bin/harness-gate-rust-source-collector`。
- 候选包 SHA-256：`b9986c787d33cd4805f00d4f490f5c679887bf74e93633148e47364cc2e10724`。
- 系列：`measurement-series/v1:4e5c32ca13421e532aad997c33701b94828b94dce0d3660e48f171f9b3a7f67e`。
- 项目后端 capabilities 和 producer/policy 绑定已改为该系列。

源码函数以 1 起计，显式 `if`、循环条件、`?`、短路逻辑、match 分支／guard、
let-else 等按版本化规则增加复杂度。嵌套 callable 单独计数；async 状态机和
宏展开产生的控制流不计入源码 CC。受支持的宏参数中的源码表达式仍需计数。
未知宏、条件编译属性及不支持的覆盖率映射会拒绝采集，不会按低分放行。
完整口径见插件 README；它不是任意 Rust 语法／工具链的通用支持声明。

采集在源码和构建输入的独立副本中运行 5 项真实 PostgreSQL／启动集成测试，
保留原始二进制及 profraw。插件重新合并原始计数并调用固定 LLVM 工具导出，
以准确源码位置绑定函数，不直接信任报告 JSON。创建但不执行的 Future 不算覆盖。

安装后的插件通过已发布 Core 0.4.5 的签名采集与后端策略判定：

- 10 个源码函数全部满足 CRAP ≤10、源码行覆盖率 ≥80%、源码 region 覆盖率 ≥80%。
- 最大 CC 为 7，最大 CRAP 为 `15428/2197`（约 7.0223）。
- 最低函数行覆盖率为 `12/13`，最低 region 覆盖率为 `28/34`，均在启动函数。
- 5 项 AST 测试、12 项测量／协议测试通过；Rust 原生样例验证 CC=10/11 及未执行 async。
- Core 拒绝旧上下文、过期请求、篡改签名、nonce 重放、篡改产物这 5 类输入。
- Core 的独立合成数值测试验证：10 通过，11 和 10.0001 失败。它们只验证策略比较，
  不冒充真实源码采集；真实采集及原生边界样例的测试分别保存。

`acceptance.json` 保存结果、精确指标、源文件摘要、包／证据摘要。
`evidence.tar.gz` 保存源快照、收据、重新导出的 LLVM、公开验签材料及 Core 报告，
不包含私钥。原始 native replay 包约 77 MB，保留在本机
`~/.local/share/codexsymphony/gate-acceptance/rust-source-rc1-native-replay.tar.gz`，
其摘要也在 acceptance.json 中；重放需要这个包及匹配工具链，不能只靠精简归档。
所有签名均为本地临时验收签名，不是正式插件发行签名。

这次仅证明后端源码系列的本地链路。前端本地链路见相邻 `frontend-rc2` 目录。
全项目 CI 验证的秘密扫描、架构检查和 7 项构建／测试步骤均通过，但仍因缺少
受信宿主的 `ci-state.json` 阻断。API 合约、完整多 collector 组合、宿主签名隔离、
正式发行与 CI 保护规则仍待验收，**完整门禁未 PASS**。

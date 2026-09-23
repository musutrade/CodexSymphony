# GH-88 原项关联修复验收

产品契约、显式范围格式及迁移/回退见 [linked-repair.md](../../linked-repair.md)。
真实验收使用隔离 PostgreSQL、真实 Runtime、GitHub App 和两个已授权的一次性仓库。
PR 的推送、创建和合并均由产品路径执行；验收读取数据库事实，不写入合并或通过结果。

## 接手与历史边界

Symphony 原 GH-88 因 4 小时运行上限阻塞。人工接手保留了原工作、失败证据和预算，
没有重置该上限。旧 B06 的初始候选基于早于 B05 修复的目标版本，PR #9 落后主干。
通过产品取消接口完成需求 2、3、4 的收尾，产品关闭 PR #9；它不是通过的 B06。

随后以当前已接受基线建立单独的 B06-R2 验收组，授权 3，需求 5、6、7。
这个更正夹具的尝试与旧尝试分别留档。B06-R2 的所有关联修复必须留在原需求 7，
保留 `validation_only`，不能新建需求来刷新修复次数或模型额度。

真实过程发现并修复了三类阻塞：取消已收尾子项仍占队列；历史 PR 周期轮询拖延
当前交付；存储保护在 Runtime 会话建立前触发时，未派发预留无法证明停止。
后续双仓验收又发现集成监督目录没有使用对应预留额度，正常文件增长触发存储停止。
相关修复保留控制锁、原工作、累计资源计费及原停止证明要求，没有放宽质量阈值。

## 真实闭环

B05 在原需求 1 内由 [disposable #7](https://github.com/musutrade/disposable/pull/7)
真实合并后失败，再由 [修复 PR #8](https://github.com/musutrade/disposable/pull/8)
合并到 `2fe3b96ff733aa7ec84006196c703ea99d3989be`，复验后 Done。
原失败身份为 `post-merge:2638386ff50a4b9a8bcb2e998555d602e5a7c61804e1161626df3ff1ee5705c3`。
该闭环使用早期候选树 `4d2f710f1ac5b9e696d1fb9ae7bcf12f7967b5ad`，不冒充后续源码的重跑。

B06-R2 的两个代码子项分别通过
[disposable #10](https://github.com/musutrade/disposable/pull/10) 和
[disposable-2 #3](https://github.com/musutrade/disposable-2/pull/3) 合并并完成子项验收。
它们给集成项提供的精确版本是 `7adb08c989a9d85f85f8250b07e6d4496bd5404a` 与
`a873effc313fa7f751e79edc88946879102f73c8`。

集成项在暂停状态下跨重启保留了任务、调用次数和额度。第一次服务启动发生连接池
超时；重新启动并经普通存储复查接口恢复。集成存储记账修复后，原调用
`integration-d53b602c-6911-44c6-8597-a10e152bf165` 保留为 interrupted，
有限基础设施重试 `integration-9b345d6d-f89b-414f-b3fe-9553769fb432` 执行真实验证并失败。
这些人工恢复均保留记录，不能将本次验收标为全程零干预。

旧失败尝试的存储分配继续累计，第二次修复预留时触及夹具原有 1 GiB 项存储容量。
隔离宿主通过新策略版本 `gh88-disposable-storage-v2` 将有限项容量改为 2 GiB，
全局仍为 8 GiB。原分配、失败调用、模型预算和两次共享修复上限均未清零或增加。
这次容量配置变更与普通接口恢复另有原件，不能声称在原 1 GiB 容量内完成。

两次修复均属于原需求 7：第一次合并 [disposable #11](https://github.com/musutrade/disposable/pull/11)，
更新后的组合仍因第二仓失败；第二次合并 [disposable-2 #4](https://github.com/musutrade/disposable-2/pull/4)。
最终精确组合为 `d8e16fd828e568e0dcf2f0de23ecd3afb904e138` 与
`b7527479849506046b7ad9b74466db7b2a65a3cb`，集成调用
`integration-9c652a2f-aa67-4153-a322-f1fd15e3918c` 通过，原需求 7 及父组均 Done，
两个关联失败均 complete。结构化事实及原件摘要见 [acceptance.json](acceptance.json)。

最终集成验证使用源码树 `7251e8108999d4cd61e89d235187c38587bb0cf0`，包含存储锁竞争修复：
已确认回滚的 PostgreSQL 55P03 仅拒绝当前动作，下一轮仍须重新通过持久化和容量检查；
未知错误、容量不足及 I/O 故障仍锁定存储保护。该行为有实际行锁/咨询锁回归测试。
本验收证明上述功能闭环，不等于已完成上线可靠性专项。

## 边界回归与复跑

真实远端闭环与受控边界测试分别记账。Rust/PostgreSQL/Git/受监督进程测试检查：

- 第三次共享修复预留被拒绝，原失败和累计资源仍保留。
- 未评审文件、第三仓及非机器可验证的范围不能获得修复授权。
- 重复失败事件/预留、暂停、冷启动、取消和无进展不会重复派发或伪造完成。
- 两仓更新后重建全部 checkout，旧失败组合和旧 SHA 证据保持不可变。
- 集成监督文件增长使用启动前预留的 hot 额度，checkout 仅预留实际生产的存储类别。

这些边界夹具可能构造数据库或远端适配器输入，不能单独证明真实 PR 合并与业务 Done。
候选 `789fe9200c0932348019df58e56afde7ac7aa98a` 的独立 Gate 完整后端回归通过 243 项；
该轮仍因准备分支覆盖率和失败重放复杂度拒绝交付，失败原件保留为 Actions
`35821117242/1`。后续补充真实准备适配器成功绑定的边界测试，并拆分保留证据读取；
17 项库测试及 Clippy 通过。最终提交的完整独立双 Gate 以 PR 检查记录为准。

```sh
python3 /opt/symphony-env/run.py cargo test --workspace --locked
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 tools/gate.py config check
```

完整原件保留在隔离宿主的 `gh88-product-acceptance/audit/manual-takeover`、
`execution` 和每个不可变 `releases/<source-tree>` 中。早期原件另保留在 GH-88 工作区
`artifacts/gh88/real-acceptance`。不提交宿主账户、会话、App 凭据或数据库连接秘密。

# 固定工作区宿主修复范围与过程记录

> 历史实施过程，不作为当前安装或清理指南。当前范围与权威来源统一见 [存储与证据保留策略入口](../quality/evidence-retention.md)。

这是当前开发流程整改的过程记录。以下各进度段仅描述当时状态；当前实施边界见 [当前开发宿主整改](current-development-storage.md)，安装和最终质量结果以仓库外宿主回执为准。

## 已成立的前提

固定源码 `/home/gem/CodexSymphony` 已绑定到 96 GiB 受限文件系统。Git 与未提交修改保留，已安装 Gate 的 repository 和 trusted_files 绑定仍匹配。固定缓存目录已经准备，但执行入口尚未接入。

## 实际发现的三层重复

1. `/home/gem/gh90-measurements/capture-stage.py` 仍使用旧 `workspaces/GH-90`，每次在时间戳目录复制或恢复 target。
2. 已安装 Gate 的 `run.py:snapshot` 每次 `git clone` 到 UUID 运行目录；`prune_build_cache` 成功后删除当前 target。外层改环境变量不能让它复用固定验证目录。
3. 已批准 Rust collector `rust-source/0.1.0-rc.7/capture.py` 在每次 output 下创建 workspace、复制输入，并将该新路径作为 cargo llvm-cov 的 manifest 路径。它显式设置 CARGO_TARGET_DIR。因此，仅设置外部 CARGO_TARGET_DIR 不足以完成固定源码和编译缓存复用。

## 有界修复范围

- GH-90 手工 capture/check/measure 入口统一使用固定开发源；测量和普通构建分别使用固定 target，禁止复制完整 target。编排输出与临时目录必须落在受限文件系统。
- Gate 使用唯一固定验证源码目录，同步新增、删除和模式并检查完整输入身份；独立 Git 元数据必须可由隔离环境读取。运行 nonce 与报告身份仍逐次独立。
- Rust capture 增加受锁保护的固定编译源码目录能力，复用普通编译产物但每次重新产生覆盖率计数、源清单、原始对象绑定及测量结果。原始证据路径必须可独立验证；不能靠重写旧 receipt 或指向后来覆盖的文件实现保留。
- 开发用 Symphony 和交互入口共用同仓库单写入锁。未接入的新任务入口阻断派发，不能同时继续向旧路径创建工作区。
- 固定缓存清理与证据收尾由宿主程序执行。缺挂载、清理故障、异常退出未收尾时拒绝下一轮。处理普通结束、失败、取消、重启以及容量耗尽。
- GH-90 Runtime、原账户与预算累计不变；完成前不恢复真实模型调用。

## 审批与验证边界

AGENTS.md 明确要求：Do not modify trusted tools from an agent workspace；不得自行改变锁、批准新工具或削弱策略。此修复必须通过宿主/collector 的受审渠道；不直接改已安装发行目录，不修改批准摘要消除漂移。

现有 docs/quality/storage-lifecycle/python-measurements.json 记录的是之前具体 Python 来源的任务级 coverage.py/Radon 授权，不是永久 collector 审批，也不能作为新增 Gate/collector 来源的测量 PASS。用户随后明确批准本次 Gate/collector 修复及 Python 任务级 coverage.py/Radon 测量扩展；不必重新请求该范围。按先覆盖率与 CRAP、后其他检查的顺序执行，仍需精确来源验收与受审安装。

复用缓存不放宽任何要求：覆盖率、CRAP <= 10、缺测失败、签名/nonce、精确 source tree 和最终完整 Gate 保持原有权威配置。宿主准备或脚本退出 0 均不构成 GH-90 产品验收。

## 恢复位置

先完成上述受审宿主修复和来源绑定验证，随后恢复 AGENTS.md 阶段 3：测量当前 GH-90 修改。通过后完成检查、部署已批准预算扩额逻辑，在原账户写入用户已批准额度，核对累计消耗与未知预留，再恢复 C1。

## 2026-09-28 实施进度

- `fixed_workspace.py`：固定验证 checkout 的归属、互斥、增删/权限同步与并发修改拒绝已实现；12 项测量用例（包含 30 轮复用），13 个生产函数行覆盖率 100%、CRAP <= 10。
- `rust_capture.py`：候选宿主采集入口直接在冻结源码路径编译，使用已有 collector 做 source inventory/discover/series，不复制新的编译源码目录。每轮保留独立原始对象和计数；7 项测量用例，12 个生产函数行覆盖率 100%、CRAP <= 10。它尚未替代已安装采集入口，也未执行 GH-90 原生捕获。
- `database_pool.py`：候选主/HTTP 两个固定测试容器槽位，文件锁互斥；Docker 日志限制 2×1 MiB/容器，PGDATA 为按现有内存权威值限额的 tmpfs；释放时停止容器，下次重启初始化整个集群，覆盖额外数据库、角色与配置的残留。9 项测量用例，13 个生产函数行覆盖率 100%、CRAP <= 10。
- 固定数据库组件还完成 3 轮真实 Docker 验证：容器 ID 不变，每轮前一轮创建的额外数据库和角色均不存在，结束后容器停止。该结果仅是组件验收，不是完整 Gate 或部署完成。
- 运行中宿主、GH-90 手工脚本、控制器及远端入口的接线与生命周期收尾仍未完成，不得把新增模块存在描述为已经覆盖这些入口。

测量与组件验收材料位于受限存储 `evidence/manual/fixed-workspace-python-*` 和 `fixed-database-live-*`；来源摘要与精确函数结果见对应 measurements.json。这些小型材料服务于当前候选验收。

## Docker 历史残留处置

盘点共 64 个容器、46 个运行中。35 个 Gate 测试容器中有 26 个运行但连接数为 0，且无对应采集/检查进程；核对镜像、挂载及卷唯一归属后全部回收，包括 3 个独占匿名测试卷。一次 Docker 客户端超时后先读回确认 daemon 已完成删除，再继续其余项，没有盲目重复删除。

清理后共 29 个容器、20 个运行中；可用内存约从 3.6 GiB 回升到 28 GiB，Docker 卷占用从 5.351 GB 降至 935.5 MB。随后组件验收新增一个固定主测试容器，验收后处于 stopped；业务数据库和非 Gate 服务未纳入此次删除。

根因包括手工 capture/check 入口缺少清理、数据库就绪失败发生在调用方 finally 之前，以及进程异常退出后缺少独立补清。固定数量、临时存储、租约、启动前重置和收尾失败阻断必须一起接入；单加 finally 不能覆盖强制终止。

## Codex 升级读回

交互 CLI 已升级为 0.158.0。仓库、environment.lock.json 和现有受管环境仍为 0.157.1，其独立安装存在且 SHA-256 匹配。未将交互工具升级擅自扩展为 GH-90 Runtime 版本迁移。

## 受控端到端验证入口

新增候选 `manual_capture.py` / `manual_measure.py`：持有共享写入锁，使用唯一固定验证 checkout 和插桩 target；数据连接由固定数据库池供给。采集前核对实际环境指纹；采集后必须重新执行原始 LLVM 导出、逐函数策略判断、独立 raw 清单和已安装保留工具注册。未完成或失败的捕获保留 pending 标记，禁止另一轮覆盖缓存/状态。入口尚未作为已安装完整 Gate 的替代品。

上述入口及 `bounded_layout.py`、修改的 `isolation.command` 均完成任务级来源绑定 Python 测量；各被测函数行覆盖率 100%，CRAP <= 10。隔离层测量曾发现挂载循环变量覆盖 compiler target，已修正变量名并重新测量通过，未带着失败结果运行捕获。

2026-09-28 08:48 UTC 已停止开发用 `symphony-codexsymphony.service` 和 `codexsymphony-remote-gate.service`，避免旧入口继续派发；停止前控制器 running 为空，宿主没有实际 capture/check worker。开发控制器停止后 unit 状态为 failed，remote Gate 为 inactive；不能把 failed 写成正常完成，恢复前须检查原因与 readiness。GH-90 产品 live 服务未停止、预算账户未改动。

目前允许的是为本次整改验证的受限诊断采集，不恢复真实模型任务。后续仍需完成完整 Gate/远端入口接入、自动收尾与发布审批绑定。各组件 PASS 不代替完整 Gate。

## 2026-09-28 固定手工入口实际验收与恢复

诊断运行 `run-561d38ee5d79` 位于受限存储 `evidence/gate/`。它使用唯一 `validation/gate/workspace`、固定 `cache/cargo-instrumented/target` 和已有 `codexsymphony-gate-fixed-primary` 容器。42 组、402 项后端测试通过；首次编译约 6 分 17 秒，完整采集阶段约 32 分钟，实际边界见运行目录 `timings.jsonl`。

原阶段保留 FAIL：测试后打包时，完整 Git 输入中的一个带字面反斜杠转义的文档名触发 collector 的 `noncanonical relative path`。未删除或改名该无关文档。修复将 collector 的原生输入限定为后端代码、测试、迁移、Cargo 清单/锁和配置，并保留完整 Git 输入清单；生产函数清单和质量门槛不变。

`recover_capture.py` 对照原冻结源码、环境指纹和仍未改写的 compiler target，逐项验证独立 raw 副本，恢复证据打包。恢复身份明确绑定原生产器和恢复程序；原始失败记录不改写。没有重复编译或重跑后端测试。独立复算得到 2,331 个生产函数覆盖率与 CRAP PASS，详见 `recovery-measurement.json`、`independent-raw-inventory.json` 和 `capture-registration.json`。该结果仅对应原冻结树，不是后续修改后的完整 Gate PASS。

`capture_handoff.py` 在测量、注册和来源一致后保存压缩 Git 源码输入（18,518,347 bytes），排除 `.git`、依赖和编译缓存，再释放 pending 标记。这样固定验证目录可以继续同步；旧测量对应源码从 `source.tar.gz` 与 `source-archive.json` 恢复，无须保留旧 checkout。独立 raw 仍保留供当前审核，跨运行自动淘汰接线尚未完成，不得据此声称证据增长已经彻底解决。

同步实现已修复无谓重写：同一提交不再强制 checkout，内容与权限未变时跳过复制。实际两轮同步中，258 个未变 Rust 文件保留原时间戳；第二轮所有 1,709 个输入文件的 inode 和时间戳均不变，工作区 inode 与编译缓存保留。结果见 `workspace-reuse.json`。

Gate 的 `verify.py` 候选已接入固定普通编译 target 与数据库 acquire/release；覆盖成功、非零退出、异常退出和错误仓库拒绝。完整 Gate 的 `run.py`、其余采集接线及受审安装仍待完成，已安装入口尚未切换。

四个旧 GH-90 手工脚本已从 `/home/gem/gh90-measurements/` 撤下，防止继续创建旧资源；17,840 bytes 原脚本及摘要保存在现有操作目录的 `retired-gh90-entries/` 与 `retired-gh90-entries.json`。跨文件系统 rename 被拒后，改用复制、校验两侧摘要、移除旧入口，未覆盖原文件。

本轮修改后的 Python 来源测量均为函数行覆盖率 100%、CRAP <= 10：

| 来源 | 用例 / 函数 | `evidence/manual/` 下报告目录 |
| --- | --- | --- |
| fixed_workspace | 13 / 13 | fixed-workspace-python-1790588040516293707 |
| rust_capture | 8 / 13 | fixed-workspace-python-1790588048498092887 |
| verify | 2 / 1 | fixed-workspace-python-1790588361903293548 |
| capture_handoff | 3 / 2 | fixed-workspace-python-1790588687878169833 |
| manual_measure | 7 / 8 | fixed-workspace-python-1790588697222649791 |
| recover_capture | 5 / 5 | fixed-workspace-python-1790588697222579183 |

固定数据库在失败退出时自动停止；恢复测量没有启动数据库。开发控制器和远端 Gate 保持停用，GH-90 live 服务与原账户未改动；真实模型调用未恢复。尚未执行最终完整 Gate、发布或部署本轮候选。


## 完整 Gate 候选接线（待最终验收）

完整 Gate 候选使用唯一验证槽位、固定编译缓存及两个有限数据库槽位。Python 测量绑定当前完整 Git 输入清单，并按 Git HEAD 的 AST 对比测量全部新增或修改的顶层生产函数；未修改的旧函数不冒充本轮测量结果，最终完整 Gate 仍必需。

远端候选只向固定仓库抓取 Git 对象，不改变开发目录的 HEAD、索引和未提交内容。受保护输入从精确提交的常规 Git blob 核对整个批准快照；只准入 execution_version 3。验证时在同一固定槽位检出该提交，依赖只读挂载。为保留 PR 间隔离边界，远端普通/插桩 target 使用两个固定独立路径并在下一任务开始前重置；本地普通/插桩缓存继续复用。远端候选暂对所有提交执行完整 Gate，尚未复用旧文档/相同树优化。

`bounded_retention.py` 使用已安装存储策略的数量、时限和字节预算。只有源码归档、注册身份及 PASS 测量摘要均核对一致且无 pending 标记的捕获，才可回收已命名 raw/binary 产物；正在运行、未完成和未知目录拒绝回收。完成身份替代按目录年龄猜测是否结束。主入口前后执行，独立 user timer 补清，timer 周期取自原已批准 timer。候选运行不会以非活动批准触发真实回收；最终验收通过并按安装渠道激活后才启用。

失败日志、测量结果、原始来源身份和紧凑源码归档保留；完整 checkout、node_modules、target 不作为证据复制。硬容量由已挂载的专用文件系统保证，缺挂载即拒绝运行。开发用 Symphony 仍停止派发；其产品级单工作区控制器由 #140 实现，不能把本轮当前交互开发 Gate 的接线声称为产品能力完成。

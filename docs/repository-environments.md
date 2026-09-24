# 每仓库环境接入（GH-119）

协议及字段语义只在 [P9/P11](extension-protocol.md#p11-仓库环境执行接入gh-119) 定义。
本增量接入 Rust 产品；没有修改开发控制器、CI 策略或生产安装，也没有启用队列。
完整验证实现替换、凭据交付适配及 local_git 闭环仍分别属于 #120/#121/#105。

## 配置与操作

1. 项目在 Repository 的可选 `environment` 字段提交一份 `environment::Plan`。
   HTTP/快照使用严格方案的 JSON 文本，与现有扩展观测文本保持一致；核心先解析
   为带未知字段拒绝的 Plan，再进行批准和身份核对。CLI 的 PLAN.json 是原生对象。
   `lockfiles` 只引用项目已有相对路径，不复制锁文件；具体工具、镜像、限额、
   并发、服务状态及检查均由受审项目扩展解释。平台数据库不是项目数据库。
2. 操作者在候选工作区和项目资源目录之外安装自包含、可执行的受审探测程序。
   `ENVIRONMENT_CONFIG` 指向宿主 `environment_host::Registry` JSON；登记包含
   实现 SHA-256、独立的规范化绝对资源目录、超时和批准的方案摘要。
   Registry 不接受两个 profile 的资源目录重叠。探测程序不得请求交付凭据。
3. 使用 `Profile::identity(evidence_root)` 计算宿主配置摘要，写入方案
   `host_profile_digest`；然后用 `Plan::contract_digest()` 填充环境绑定，最后
   用 `Plan::digest()` 得到整份方案摘要。将最后一个摘要加入宿主 profile 的
   `approved_plans` 是操作者的评审动作；产品不安装工具、不自动更新这些值。
   这些函数使用 UTF-8 JSON 的确定性序列化和 SHA-256；不要手工重排摘要输入。
   产品登记的 scope_ref 为 `repository:<内部仓库 ID>`，repository_revision 为
   `repository:<内部仓库 ID>@<本次仓库版本>`；不能把同一 profile 借给另一个仓库。
   撤销授权不要求先修好环境；恢复启用则需重新评审绑定。
4. 仓库配置接口在启用时核验 dev/test，评审快照保存整份方案。服务启动、
   每次准备（含修复/回答恢复）、实际 Agent 启动、候选验证、交付写入前和启动
   恢复闸门均执行适用核验。历史快照缺少 `environment` 时保持原有准备路径。
5. 本地/CI 可运行以下无数据库入口。`PLAN.json` 是已经批准的同一份方案，
   进程从 `ENVIRONMENT_CONFIG` 读取同一类受审登记；不能从 Agent 参数选择程序。

   ```sh
   codexsymphony-server --environment-check PLAN.json validation
   codexsymphony-server --environment-check PLAN.json ci
   ```

   `validation`、`delivery`、`ci` 共用 test 角色；`ci=false` 时输出
   `not_applicable`，不执行 CI 探测。dev/test 差异必须在方案里解释。
   资源级 CLI 成功只说明环境一致，不签发任务验证证据，不代替项目测试/覆盖率。

`apps/server/tests/fixtures/environment/probe.py` 是受控测试扩展示例，分别探测真实
Python/Node 工具、配置文件与 Unix socket 服务。它对镜像、内存和并发使用可变
测试配置模拟观测，**不是**生产容器/资源探针。实际项目必须用自己的受审探针
读取有效配置、真实进程/容器以及资源控制器；不得直接回显期望值充当观测。
本仓 `tools/environment_contract.py` / sccache 仍只是 Rust 项目自身的配置实例。

## 故障、观测与恢复

每次探测保留 input、launch、进程身份、stdout/stderr、退出状态、静止回执、
规范化 `actual.json`、摘要及 report。任务调用另有 `admission.json` 和
`environment_observation`；操作详情的 `environments` 包含实际/期望差异与证据路径。
其中 `report` 为原始结构化报告的 JSON 文本。
扩展替换、宿主配置替换、缺项/错身份/迟到/超量输出、未知退出均拒绝准入。
新配置文件和新安装程序正确但旧进程仍在运行，也不能通过。

环境失败不进入代码修复分类，不启动模型、不升级模型、不重建预算。
准备失败进入原有准备台账；验证/交付入口在产生验证执行或远端写尝试前拒绝。
未知探测执行先核对同一目录的 identity/quiescent 回执，不能删除目录绕过；
没有完整静止证明时保留环境阻塞。修正环境后沿用原任务/授权及适用的既有
显式恢复入口；更新批准配置必须重新评审变化部分，不得给历史结果重贴身份。

通用阶段耗时复用 `operator_phase` 等执行记录。环境耗时来自核心单调计时器，
以 `metrics.environment_samples` 输出。当前执行适配未提供构建/测试/覆盖率子阶段
或缓存命中率时为 `unknown`；未启用缓存时命中率为 `not_applicable`。
项目验证仍从原验证执行、原始输出及适用项目覆盖率采集，不从缓存推断成功，
不要求业务代码埋点。既有模型 token 缓存统计与构建缓存统计保持独立。
`stage_samples` 从准备历史、验证开始/完成记录和交付尝试采集耗时；原有阶段记录
继续提供排队/执行时间。`ci_wait` 只在已有执行记录提供 CI 等待秒数时输出 known，
否则按批准 CI 配置输出 unknown/not_applicable。历史没有起止时间时保持未知。
准备阶段环境调用受原准备截止时间约束，不能用新的探测调用延长原预算。

## 缓存与清理

`cache` 缺省或 null 表示关闭；关闭不创建缓存目录、不要求 sccache 或缓存统计。
启用时目录为批准资源根下 `cache/<identity>`，scope 必须等于仓库 host profile。
平台核对身份、写权限和实际字节容量；超限保留内容并阻断下次适用准入，不自动
清空缓存。容量检查是准入检查，执行期间的硬磁盘限额由受审宿主存储配置提供。

可选种子由操作者放在证据根下 `seeds/<manifest-sha256>`，所有目录/文件必须
只读；manifest.json 是相对文件名到 SHA-256 的映射，须完整覆盖种子内容。
平台不生成、修改或自动推广种子，工具是否使用种子由项目适配器决定。
本阶段继承可信开发环境；POSIX 权限核验不声称提供恶意同 UID 进程隔离。
需要强制写边界的部署应把种子以只读挂载暴露给执行环境，由独立受信维护路径更新。

删除缓存前暂停相关领取，确认所有使用该作用域的进程已静止，保全任务证据并
确认缓存不是唯一产物；只能清理选定 identity 目录。种子需确认所有引用已退役。
清理缓存不删除 environment_observation、原始报告、预算或未知执行意图。

## 迁移、升级与回退

迁移 0032 增加环境观测表/索引及验证/交付的可空起止时间，由记录状态转换采集；
没有回填历史耗时、授权，未变更预算或重写旧快照。
旧配置序列化不增加空的 environment 字段，因此旧身份保持稳定。
新配置先在受控资源上核验，通过当前仓库完整 Gate 后才可另行申请部署。

升级工具/配置时先暂停相关任务、静止并保全，安装操作者批准版本，核对实际
进程已经切换，再评审新方案。旧授权不会因宿主新增 approved_plans 自动升级。
回退采用能读取 environment 字段及迁移 0032 的兼容版本，并恢复与原冻结方案
匹配的宿主 profile。早于本增量的二进制不能解析新方案，不可拿它继续这些任务；
可保留暂停和证据后切回兼容版本。不得删除新字段、历史结果或预算来制造可恢复状态。

## 验收边界

`apps/server/tests/environment.rs` 使用两个独立本地项目（Python、Node），执行
真实工具探测、真实测试命令、真实 Unix socket 进程和 PostgreSQL 平台记录。
覆盖缓存开关/容量/权限/种子内容，环境差异、同角色 CI、冻结快照、预算不变、
安装/进程/有效配置漂移、缺项/伪造证据、超时与未知执行保留。
外部 CI、GitHub 写、真实生产安装和完整两仓业务任务不是这组受控测试的结论；
后续闭环沿原顺序实施。本仓完整 Rust、Clippy、格式和已安装完整 Gate 结果见
`artifacts/gh119/` 与 `.symphony-evidence.json`，不以本地结果宣称远端双检查通过。

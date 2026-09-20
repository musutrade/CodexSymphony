# GH-24 单仓集成验收

真实 A01 已完成浏览器评审 → 原需求恢复 → 锁定模型执行 → 独立验证 → 产品 App 创建
[disposable PR #4](https://github.com/musutrade/disposable/pull/4)。Requirement 1/revision 1
状态为 **Submitted**，不是业务 Done；测试 PR 保持未合并。A13 未实施，不宣称完整 0a 发布。
本开发任务的精确提交双 Gate 与合并另由 CodexSymphony 的 PR/受信宿主记录。

## 可核验产物

- [真实运行、消耗与交付](real-a01.json)：最终 Run `385911f6-69eb-4ca3-a80f-d2c37e72106c`，
  candidate `a8e741155953ee97dc01b3aae1e7e77747b5277c`，validation
  `validation-385911f6-69eb-4ca3-a80f-d2c37e72106c`，PR head 与候选完全一致。
- [独立验证日志](validation.json)：受保护入口检查 marker 精确字节，并在独立候选工作区执行
  `cargo test --locked a01_marker -- --exact`，实际 1 个测试通过，exit 0。日志 SHA-256、
  入口/配置/源码身份在真实运行记录中。没有采用模型自报作为验证结果。
- [部署摘要](deployment.json)：83 个服务源码/迁移/锁文件与部署副本逐字节匹配；
  基于 `e7429d216f1d996d153503d22b4e2d59d59dc63f` 的保留实现，迁移至 0015。
  外部持久 PostgreSQL、3081 API、4200 前端、Codex 0.154.0 / gpt-6-astra。
  配置只保存摘要，App/模型/签名凭据均在工作区外。
- [预算及授权](budget.json)：5 次真实模型调用，原预算及所有调用未重置；
  三次显式增量授权后上限为 500000 tokens / 5 turns / 4500 model seconds。
  中断调用的未知用量保留未知和保守预留，不能把界面 null 当作零。
- [真实浏览器评审](real-review.png)、[桌面详情](operations-desktop.png)、
  [手机详情](operations-mobile.png)。后两张来自确定性业务 UI 场景，不冒充真实 PR 页面。

目标仓库为用户授权的 `musutrade/disposable`（1360824360）。准备基线
`aa6f08dcd4b36d7f2072fdd408b4a33469297251` 是远端 main `0b0dd251...` 的后继提交，
增加最小 Rust 测试项目；PR 包含该准备提交。原 Requirement 从未重建。原 Run
`e6599814-1c3b-479f-bcc8-a8fd1070ea68` 和所有后继工作区、快照、协议原件仍保留。
运行期间发生的真实失败也保留在部署与验收目录，不删除失败样本。

## 集成缺口与修复

1. Runtime idle poll 只超时等待 transport，不取消已取帧的持久化处理；慢数据库和真实
   锁定 Codex + 本地延迟 provider 验证不丢结束声明。后者不计入真实模型 A01。
2. 启动容量检查移出已持有的执行锁，持锁后再次校验授权。短时 PostgreSQL 锁竞争只在
   获取锁、尚无业务副作用时最多尝试三次，每次仍为 500ms；永久竞争仍停止，业务写入不重放。
3. 存储计量只读 socket 元数据；临时 ENOENT 最多三次完整重扫。错误脱敏、重试期限和
   原件保全继续有效。0013 把阶段通知改为实际变更行触发，零行/相同状态更新不再唤醒扫描。
4. 0012 保存 storage_resume_requested：先记录恢复意图，再等待快照，已解除的 guard 也可
   重检修复遗漏。0014 归档旧实例已准备但未派发的任务，新实例重新准备新身份；派发过的任务
   不能重复使用。仅有本地提交、没有完成声明的工作可恢复；验证/交付按实际阶段接续。
5. 预算达到最后一个已授权 turn 时允许该调用完成；新调用仍被拒绝，token/时间硬上限保持。
   原逻辑在第一次用量事件时中断最后一轮，已先复现再修复。
6. App Actions 来源显式 `branch_from_pr: true` 后绑定当前同仓 PR head；固定 App、workflow
   ID/blob SHA、event、head、suite/job/attempt 校验保持。原 `test-job` 只 echo，用于来源观察，
   不充当 a01_marker 测试。Runtime 最小配置禁用无关插件自动准备，并预检实际 rg 依赖。
7. 推送子进程只继承服务代理变量，解决清空环境后无法访问 GitHub；凭据仍由 App 持有。
   两次推送超时后第三次成功，发布写入额度已用完。0015 和版本化 `delivery_recheck` 显式
   授权下一组三次写入，保留累计 attempts；第四次仅创建 PR。身份冲突不能由此清除。
8. A01 原生 fetch 使用 `.ok` 属性；只读 `--resume` 校验原仓库/revision，拒绝重复 PR。
   集成入口复用现有故障测试，E2E 夹具独立登记仓库，不依赖其他测试先执行。

首次两次恢复调用没有完整用量，现场旧日志不足以唯一归因；不能把所有中断都归结为
某一个已复现缺口。测试 PostgreSQL 的 EOF 后续由容器 OOM 证据确认，测试夹具容量增至
2 GiB；持久产品库未重建。锁争用采样、修复前失败及历次采集均保留。

## 验证与有限试用

最终检查结果见 [本地验证](local-validation.json) 和 [原生 Rust 测量](rust-quality.json)。
普通集成入口通过 98 项 Rust 测试、fmt、Clippy、观察脚本、19 项前端单测及 lint/build、
14 项桌面/Pixel 7 E2E/axe，以及 Gate config/secrets/audit。原生测量使用最终输入重新采集，
940 个源码 subject：函数覆盖率 100%，最低行/region 覆盖率 80%，最高 CRAP=10；
104 个输入摘要与最终工作区一致，零违规。开发测量不冒充独立签名 Gate。

本次从 Ready 到 PR 共 33286 秒，包含部署排障和等待；产品记录 8 次介入
（7 次 storage_recheck、1 次 delivery_recheck），另有预算增量授权、部署及诊断操作。
人工实际时长和费用未测量；界面 human_seconds=0 不能解释为无人介入。零介入样本为 0/1。
模型修复计数为 0，基础设施排障不伪装成一次代码修复。

A01–A12 引用、停止/恢复步骤及后续五条样本模板见
[单仓验收](../../single-repository-acceptance.md)。本样本证明有介入恢复后的真实闭环，
不证明无故障吞吐或多仓能力。PR Submitted、远端 CI、合并和业务 Done 保持分离。

原始证据：`artifacts/gh24/`、外部部署 `a01-gh24/recovery-deploy-{2,3}/`、
`a01-gh24/execution/`；当前旧检查点保留供追溯，以本文和绑定摘要的最终记录为准。

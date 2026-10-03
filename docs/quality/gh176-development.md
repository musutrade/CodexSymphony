# GH-176 本地开发记录

任务：PR 发布重复验证与候选准入维护解耦。开发基线是已合入 PR #175 的 main `d09e334b017b6ac5704b9439d755b67e6466df7a`；工作区 `/home/gem/CodexSymphony`，分支 `feat/gh176-publication-validation-reuse`。开始时工作区干净，原 GH-90 分支保留。本轮范围为现有 Python 开发/发布工具、受控 fixture 与配套流程说明，不新增业务模型调用。

前置只读检查确认：environment.lock 与实际工具指纹匹配；已安装 runtime pins、config files、trusted files 与基线一致；批准身份 `4cb9798e83d0bdf90a5dd1a0fe7d4f1807c9588492f6c6986e91272795e9cf02`，执行环境指纹 `5efcbf80f543d4378df0a6cd4523952fa021c2e97200484be983a0cc543b3099`。验证 checkout 的 `.git` 为目录，独立于开发目录外部指针；pending capture/gate 均不存在。node_modules 已准备，固定数据库允许宿主按现有批准供给。磁盘、内存有可用容量。这些检查不是质量 PASS。

实现和验收边界见 [验证复用协议](gh176-validation-reuse.md)。新协议默认关闭；审查与安装批准由宿主操作员负责，feature 工作区没有部署或自批契约。本轮本地测量、检查和完整 Gate 的真实状态仅由 retained capture/measurement、timings 和 Gate report 判定；本记录不把 fixture 结果写成现场成本验收或受保护 CI 成功。

首轮 native capture 和独立测量留存在 bounded run `run-49580531c64e`：后端 2,487 个函数覆盖率/CRAP PASS，前端按既有 subject 分组为 27 个文件、1 个声明文件、234 个函数，覆盖率/CRAP PASS。frontend 原始材料独立保留在该 run 的 `probes/frontend-probe`。publication 40 个回归通过；remote 128 个回归中 1 个 inherited fixture 的 pin 总数断言失败，原日志保留。随后修改绑定的有限保留、远端输入核验与相关 fixture；首轮证据不授权修改后的代码，必须重新测量。后续状态以新 run 的实际报告为准。

第二轮 retained run `run-afaf8869ab5b` 的独立测量为 PASS：后端 2,487 个函数、前端 234 个函数，违规项均为空。阶段日志与原始 capture 保留在该 run。format、secret scan、Clippy、frontend lint/build 通过；Clippy 首次临时检查启动器的临时目录不可达，修正为最小环境及已挂载的编译缓存后通过，源码未变。selftest 的 publication 43、remote 131、quality-host 147 个测试通过，随后安装器测试发现 release 清单尚未列出新增的 validation.py。修正该预期清单后，恢复条件为新源码完整同步、重新 source-bound 测量 PASS，再运行检查；不沿用第二轮测量授权修正后的测试代码。

恢复补充：测试清单编辑时曾产生缩进错误，在启动后立即只读发现；`run-aa6f733a6416` 采集进程已停止，阶段 FAIL 日志及 pending 原件保留在该 run 的 interrupted-pending-capture.json，未登记为完整捕获、未清理原始材料。修正缩进后在取得现有槽位锁的条件下移出该 pending 标记，下一轮使用全新目录；无已安装宿主代码修改。

修正后的候选在 retained run `run-47ef1f700358` 完成新 capture、独立原始清单、宿主登记和 source-bound 测量；后端 2,487 个函数及前端 234 个函数的覆盖率/CRAP 均 PASS、违规项为空。随后完整 gate.selftest、format、Clippy、secret scan、frontend lint/build 全部通过，命令日志和阶段耗时保留在该 run。文档在完整 Gate 前冻结；最终完整 Gate 是否通过须读取已安装宿主创建的新 run/report，不由本段断言。新协议没有安装，现场成本对比、服务 PID/start time 验收及受保护 CI 仍待独立审查和批准后的真实环境执行。

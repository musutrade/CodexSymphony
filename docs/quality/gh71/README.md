# GH-71 平台认证验收

本项在分配工作区、一次性 PostgreSQL 16 和临时证书 HTTPS 入口验收。账号与密码由测试随机生成，真实执行宿主管理命令、Argon2id 验证、数据库会话、Cookie 和 CSRF 流程。既有业务测试也通过真实账号登录，不含生产认证绕过。

## 验收映射

| 契约 | 实际证据入口 |
| --- | --- |
| AAuth01 | `health` 中发现实际业务路由并逐方法检查匿名 401，比较业务表前后快照；浏览器桌面／Pixel 7 同账号读回同一草稿；受保护页面进入登录；管理命令与公开端点清单测试 |
| AAuth02 | `health` 的统一错误、账号／可信来源双维持久限流与伪造转发头测试；`auth_session_acceptance.py` 在真实服务重启后复验两维限制 |
| AAuth03 | 独立盐与摘要存储检查、Cookie 属性、注入时钟精确检查八小时绝对期限；HTTPS 实际退出、改密／重置撤销与跨重启旧会话重放拒绝 |
| AAuth04 | 登录／退出／业务写的跨站、缺失、固定旧 header、错误及其他会话证明拒绝；正常请求成功；真实进程日志按生成的密码／会话／证明扫描，浏览器检查持久化存储与 HttpOnly |
| AAuth05（部分） | 无 Access 身份即可平台登录；仅身份头不能访问业务 API，未知代理不能改变限流来源；本项不覆盖公网源站隔离 |
| AAuth06 | 401 后重新登录、恢复当前页面未提交输入且不重放写请求；拒绝外站、协议相对及编码返回地址；重置后新密码成功、旧密码与会话失败 |

手机证据使用真实 Chromium 的 Pixel 7 项目与移动视口，不宣称实体手机或真实公网部署。浏览器生成草稿回归沿用明确标注的协议夹具，不作为真实模型调用验收。Runtime 的既有真实集成由完整 Cargo 测试运行。

## 源码与证据

交付时工作区根目录 `.symphony-evidence.json` 以 SHA-256 列出 `artifacts/` 下的原始日志、测量、截图、源码清单与发布记录。`publication.json` 记录 GitHub 返回并读回的真实 PR head；本机 HEAD 可以仍是控制器基线。原生覆盖率输入清单与前端原始源码清单用于核对实际被测内容。

所有结果为本地验收；独立宿主在发布的精确提交上执行完整 Harness-Gate 和 Trusted Harness-Gate。生产部署、公网 HTTPS、源站隔离和最终集成仍须由 M2-2／最终集成项提供真实部署证据。

## 本地验收与宿主恢复

完整 Cargo 回归（含真实 Runtime）报告 138 项通过（含子进程套件），零失败／忽略；fmt、Clippy `-D warnings`、全目标编译和构建通过。新增适配器故障测试验证存储错误响应不泄露细节；生产认证仍统一保护业务路由。端口占用测试验证服务拒绝启动且不启动后台工作。

最终原生采集覆盖 89 个生产源码文件、1256 个函数，全部输入摘要与工作区一致，覆盖率／CRAP 无违规。此前 9 项违规通过拆分启动函数和补充真实错误路径测试修复，未改变阈值或豁免。原始对象、计数器和采集快照保留在 `artifacts/gh71-completion/native-capture.tar.gz`；测量结果是本地诊断，不是签名 Gate。

Angular 43 项单元测试、lint、生产构建通过；最终前端源码及测量配置与已通过版本逐字一致，23 个生产文件／164 个函数无质量违规，核对记录见 `artifacts/gh71-completion/frontend-source-verification.json`。构建存在初始包 552.11 kB 超过 500 kB 警告阈值的提示；未修改阈值。

受控 HTTPS 的桌面／Pixel 7 浏览器验收、跨进程重启会话验收及实际 API 合约测量的完整日志和结果分别保留在 `artifacts/gh71-completion/` 与 `artifacts/gh71-product-contract/`。AAuth 映射见上表；不将移动视口测试描述为实体手机或公网源站隔离验收。

HTTP collector rc.5 与 Argon2 Cargo 管线的后端 measurement series 均已有独立宿主的安装／审批回执，分别见 `artifacts/gh71-interceptor-recovery/` 与 `artifacts/gh71-backend-series-recovery/`。本次保留宿主提供的 series/capabilities 绑定，不修改门禁策略、requiredness、覆盖率或 CRAP 阈值。

Gate 配置、架构审计和待发布源码的秘密扫描通过。工作区包含历史原生证据大压缩包，普通全工作区秘密扫描触发文件大小限制；正式待发布文件使用 `secrets --staged --json` 检查，原始证据不进入 Git 树。此前环境恢复与失败日志继续保留，不覆盖为成功。

所有本地证据通过 `.symphony-evidence.json` 的精确 SHA-256 清单交接。发布记录绑定 GitHub 返回并读回的提交与 Git tree；精确提交的 Harness-Gate 和 Trusted Harness-Gate 仍须由独立宿主运行。生产部署、公网 HTTPS 和源站隔离仍属 M2-2／最终集成。

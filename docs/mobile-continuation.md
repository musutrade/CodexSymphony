# 手机接续与隔夜回答（GH-73）

电脑和手机使用 M2 平台账号，操作同一份 Requirement、草稿、整组授权和队列。
手机没有独立调度器或授权记录。保存草稿不执行；在 `/drafts` 保存或选择草稿后，
进入“评审覆盖、策略与整组预算”，核对父子范围、AC 覆盖、依赖和预算，再整组确认。
队列页可查看父子进度，并按既有版本协议编辑、重排及重新评审未开始项。

`/inbox` 保存业务待办；需求详情提供问题回答、暂停、恢复和取消。
通知未送达或页面离线不删除待办。页面重新连接后读取当前版本，写入失败保留上下文。
问题版本更新后不会把旧版本的未提交回答自动带入新问题；回答本身不修改需求范围。
范围变化须回评审，旧版本回答及重复提交不能授权旧 Run 继续执行。

普通业务问题的身份独立于 app-server RPC、线程和 Run。等待两小时或 Run 八小时超时后，
旧执行组停止并保全；仍有效的问题可以隔夜回答。全部问题已回答、原范围仍获授权、
未暂停或取消、预算与预检通过时，服务使用新 Run 还原原工作，不要求先恢复旧 Run。
后续再暂停或恢复时，输入按同一需求修订携带有效回答，历史 `resumed_run` 链接不被当作回答有效期。

恢复任务在还原中或预检后遇到服务重启时，旧任务存入 `runtime_resume_history`，
保留原始快照、已有工作目录和预算，再以新身份预检。迁移 `0023` 只扩展历史记录的阶段约束，
不删除或改写草稿、授权、消耗、问题或工作保存事实。暂停意图仍独立生效；重启不能解除暂停。
取消沿用停止、保全和外部动作对账契约，收尾完成才释放队列；不制造 Done 或回滚事实。

## 可复现验收

在已提供的隔离开发环境中串行运行涉及服务的验收。产品固定实例锁禁止同一环境同时启动
多个服务；并行运行浏览器服务与 Rust 服务重启测试会收到 `WouldBlock`，不是数据库故障。

```sh
python3 /opt/symphony-env/run.py cargo test --workspace --locked
cargo fmt --all -- --check
python3 /opt/symphony-env/run.py cargo clippy --workspace --all-targets --locked -- -D warnings
# web/angular 内：npm run lint；npm test -- --watch=false；npm run build
python3 /opt/symphony-env/run.py python3 tools/auth_browser_acceptance.py
python3 /opt/symphony-env/run.py python3 tools/mobile_recovery_acceptance.py
python3 tools/gate.py config check
```

`mobile_recovery_acceptance.py` 使用真实服务进程、PostgreSQL 隔离 schema、Git 工作区、
进程停止回执及 HTTPS Chromium。桌面创建并评审后关闭浏览器；仅把 fixture 的
`runtime_session.created_at/waiting_since` 和旧 `agent_run.created_at` 向前移 90000 秒，
使等待和 Run 均超时。停止保全后实际重启服务；390×844 浏览器回答，自动产生第二个 Run；
手机暂停后再次重启，确认仍只有两个 Run；显式恢复产生第三个 Run，仍带原问题回答；
最终取消并对账。截图和 axe、问题版本、原/新 Run、工作清单、授权与每次预留/消耗记录
保存在 `artifacts/gh73/recovery/`，清单由工作区 `.symphony-evidence.json` 绑定摘要。
新 Run 必须实际写出匹配当前 cwd 的进程证明；仅还原出旧证明文件不算已执行。

该恢复场景使用明确标注的脚本 app-server 对端和合成 GitHub 能力观察，
以确定性触发问题及保持进程运行；不宣称真实模型或 GitHub 交付验收。
预检执行实际工具、文件写入和命令；完整 Cargo 测试另含 pinned Codex 的 `runtime_real`。
浏览器回归覆盖自然语言/导入草稿、父子整组授权、AC/预算、未开始队列编辑/依赖、
版本冲突、同账号跨端和持久待办。取消已有 PR、合并竞态及取消不放行依赖的契约
由既有 delivery/group_queue 集成测试覆盖，不使用真实 GitHub 副作用。

完整精确提交 Harness-Gate 与 Trusted Harness-Gate 由发布后的独立宿主执行。
本项不表示 M2 已远程上线，也不实现 M3 自动合并、CI 自动修复或 validation_only 执行闭环。

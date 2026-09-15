# GH-12 本地实施验收 · 2026-09-15

基线 `fee94e5e6f7fa9af0c952597fc649e7833f54ba5`，分支 `symphony/GH-12`。
这是该基线上的**新增工作区输入**验收，不能把基线 SHA 当作新代码 SHA。
[local-results.json](local-results.json) 保存输入文件 SHA-256、实际夹具容器 ID 和结果。
最终发布 SHA 以同仓 PR head 及宿主 API 回读后的 `.symphony-handoff.json` 为准。

## 已运行

| 命令 / 检查 | 实际结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `python3 /opt/gh12-env/run.py cargo test --workspace --locked` | PASS，10 项集成测试（含健康 200/503、启动/退出、配置和请求边界） |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo check --workspace --all-targets --locked` | PASS |
| 前端 `npm ci --offline --no-audit --no-fund` | PASS，原锁文件安装 |
| 前端 `npm run lint`、`npm test -- --watch=false`、`npm run build` | PASS，6 项单元测试 |
| `python3 /opt/gh12-env/run.py python3 /opt/gh12-env/e2e.py` | PASS，Desktop Chrome / Pixel 7 共 6 项 E2E |
| `python3 tools/gate.py config check` | PASS |
| `python3 /opt/gh12-env/run.py env -u DATABASE_URL python3 tools/gate.py verify --profile ci --all` | 秘密扫描、架构检查、7 项执行步骤 PASS；完整质量因缺少宿主 `ci-state.json` 阻断，退出 1，**不是 Gate PASS** |
| `python3 /opt/gh12-env/run.py python3 docs/quality/gh12/probe-fixtures.py` | PASS，配置渲染、卷保留、tmpfs 丢弃、两库隔离，临时表已清理 |

`run.py` 会把 DATABASE_URL 和 TEST_DATABASE_URL 都设为隔离测试夹具；Gate 正确拒绝两者相同，
所以 Gate 命令显式移除 DATABASE_URL，测试仍使用 TEST_DATABASE_URL。
原 `tools/gate.py hook` 尝试写受保护 Git index，按沙箱约束失败；未改变 Git 权限。
上表直接 verify 在当前工作树检查；未提供或复用签名输入，CI 完整接受仍由宿主完成。
最初秘密扫描识别了开发示例 URL，示例已使用明确 `example-local-password` 占位值，不改变扫描策略。

## 新行为证据

### 请求边界

`health` 目标包含 `tests/support/security.rs`，因此既有受信宿主指定的 health/startup 采集也执行新负例。
验证预期 Host/Origin、不同来源/端口/scheme、null/缺失/重复头、非 UTF-8 头、矛盾 absolute URI、
伪造转发头、POST/PUT/PATCH/DELETE/未知方法和 CSRF 头缺失/错误/重复。中间件没有返回 CORS 授权。
写路由仅存在于测试；#13 仍必须接入真实写请求并证明拒绝时没有业务副作用。

### 源码测量

通过已安装的 `rust-source-risk 0.1.0-rc.1` 对新源码执行真实 LLVM 采集和独立重导出：

```sh
python3 /opt/gh12-env/run.py python3 /home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.1/capture.py \
  --repository "$PWD" --output "$PWD/target/gh12-rust-capture-v4" \
  --target-dir "$PWD/target/gh12-coverage-build" --source-root apps/server/src \
  --input Cargo.toml --input Cargo.lock --input rust-toolchain.toml --input apps --input migrations \
  --manifest apps/server/Cargo.toml --test health --test startup
```

从 bundle 提取 request 后执行该插件 `plugin.py collect --request .../request.json`。
24 个源码函数均可测量，CRAP 最大 `19854/2197`（约 9.037），每函数最低行覆盖率 12/13、
区域覆盖率 30/37、函数覆盖率 100%。没有改阈值或排除新代码；不支持的 `format!` 改为标准字符串拼接，
常量错误转换用 `Result::or` 避免 LLVM 消除无独立映射的常量闭包。
这些是**未签名本地测量**；保留了原始对象、profraw、receipt 和源摘要，不替代宿主完整质量判定。
TypeScript/HTTP 的受信新测量和双检查仍由宿主对发布 SHA 执行。

### 干净副本、样式与页面

从 Git 文件清单导出源码（含新增文件，不含依赖/缓存/构建输出，无源文件软链接）到
`target/gh12-clean`，重新离线 `npm ci`。Node `--permission` 未授权 arc-admin，
显式读取其 token 文件返回 `ERR_ACCESS_DENIED`；Angular production build 退出 0。
因 Node 的文件复制会检查所有输出祖先目录，构建输出指定 `/tmp/gh12-clean-build-output`。
构建使用 worker/原生依赖，故此检查只证明样式构建自包含，不是新增执行器隔离能力。
生成 CSS 与常规构建 SHA-256 相同，源码 token 与固定来源 SHA-256 相同。
构建脚本和完整日志在 `target/gh12-clean-build.cjs`、`target/gh12-clean-build.log`。

E2E 验证 1200px 最大宽度、24px/16px 内边距、无横向溢出、主要操作 >=40px、
跳转主内容、Tab/Enter 重试、可见 outline、加载禁用、空响应和网络失败重试、axe WCAG AA，
以及减少动效/强制颜色。已查看桌面与手机截图，标题、卡片、操作和错误文案无裁切/遮挡；
焦点与状态文字可辨认。截图保存在 `web/angular/test-results/`，完整日志在 `target/gh12-e2e.log`。
页面没有提交表单，业务空列表、提交中和关联表单错误由 #13 随实际页面验收；不宣称登录或完整视觉平台完成。

### 数据与启动

Compose 经实际 CLI 渲染并核对命名卷/tmpfs、数据库角色和端口。夹具由宿主固定 broker 管理，
并非 Agent 通过 Docker 启动新容器；宿主自有容器名、网络和合成密码。测试只在这两个明确授权的夹具创建随机临时表。
[probe-fixtures.py](probe-fixtures.py) 可重跑，但必须与其他数据库测试串行。

配置示例的库名、用户、地址、端口和 WEB_ORIGIN 用于新编译二进制启动，仅将密码和 sslmode 参数
替换为固定夹具值；真实开发夹具返回健康 200，SIGINT 正常退出。记录在 `target/gh12-example-startup.json`。
这不是对真实用户库、灾难恢复、业务 migration、Run 持久化或远程认证的验收。

## 剩余边界

完整 `Harness-Gate` / `Trusted Harness-Gate` 由控制器在 PR 发布后对**精确 head**重新采集和观察。
本记录不包含任何自签/复制的受信输入，不以旧环境 PASS 替代新代码验收。
0a 仍只交付 localhost 基础与未来 PR 闭环，用户名/密码登录在 M2；本项没有业务写接口、调度或三真相状态转换。

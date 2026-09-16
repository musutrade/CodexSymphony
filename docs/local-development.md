# 本机开发与验证

当前骨架：Rust 1.97.1，Axum 0.8，SQLx 0.8，PostgreSQL 16；Node 24.18.0，
Angular 22，Material 22，TypeScript 6.0.2。精确依赖见 Cargo.lock 与前端 package-lock.json。

## 干净检出的工具链

`rust-toolchain.toml` 选择 Rust **1.97.1**（含 rustfmt/Clippy）；`.node-version` 选择
Node **24.18.0**，`web/angular/package.json` 的 packageManager 固定 npm **11.16.0**。
先用本机版本管理器安装/选择这些版本，例如 `nvm install "$(cat .node-version)"`，
再核对 `rustc --version`、`node --version`、`npm --version`。依赖只从已提交锁文件安装。

```sh
export CARGO_TARGET_DIR="$PWD/target"
rustup show active-toolchain
cd web/angular
npm ci
```

项目开发环境已预装版本和离线缓存，使用 `npm ci --offline --no-audit --no-fund`。
不需要安装 arc-admin，也没有对另一个源码目录的构建依赖。

## 持久化开发 / 内部试用启动

在仓库根目录执行；示例只有合成的 localhost 凭据，不包含真实密钥。API 读取进程环境，
不自动加载 `.env`。数据库密码和连接 URL 必须一致；自选密码含 URL 保留字符时须编码 URL。

```sh
cp .env.example .env
# 首次初始化前按需要编辑 .env；不提交此文件。
docker compose --env-file .env -f docker-compose.dev.yml up -d --wait postgres
set -a
. ./.env
set +a
export CARGO_TARGET_DIR="$PWD/target"
cargo run --locked -p codexsymphony-server
```

另一终端执行 `cd web/angular && npm ci && npm start`，打开 **http://localhost:4200**。
健康接口为 `http://127.0.0.1:3081/api/health`。Angular 代理 `/api/**` 至 API，
重写 Host 为目标地址并保留浏览器 Origin。直接使用 127.0.0.1 打开前端时，把
`WEB_ORIGIN` 改为 `http://127.0.0.1:4200` 后重启 API；预期 Origin 必须含实际端口。

API 启动前校验 DATABASE_URL、数字 IP:port 的 BIND_ADDRESS、WEB_ORIGIN。
BIND_ADDRESS 仅允许回环 IP，默认 `127.0.0.1:3081`；测试可用端口 0。
WEB_ORIGIN 仅允许 HTTP 的 localhost、127.0.0.1 或 [::1]，不接受用户信息、路径、查询、
片段、通配域和非法端口。校验失败不连接数据库、不宣布就绪；配置错误不回显数据库 URL。
随后连接数据库、执行 SQLx migrations，最后开始监听；`0001_requirements.sql` 创建首批仓库、需求、修订、控制请求和事件记录。

### 需求工作台

打开 `/requirements` 登记首个可信仓库及预算，再填写需求、验收条件和验证步骤。
验证步骤选择 `cargo_test` 或 `npm_test`，填写测试选择器、预期结果和超时；允许引用将新增的测试。
保存 Draft 后可编辑，再点击“评审已保存版本”与“确认评审并 Ready”。Ready 冻结已保存输入及展示的仓库策略，
撤回后才可继续编辑；旧修订保留。浏览器关闭不删除记录。

当前运行、仓库交付和部署网络检查均未就绪，Ready 只表示持久化排队。联网声明仅表达意图。
仓库后续策略修订和安全撤销可通过 [业务 API](../api/README.md) 完成；网页用户名/密码认证按 M2 实施。

### 持久目录与初始化

| 用途 | 当前路径 / 行为 |
|---|---|
| 开发数据库 | 独立项目 `codexsymphony-persistent-dev` 的命名卷 `codexsymphony-persistent-dev_postgres-data`，挂载 `/var/lib/postgresql/data`；端口 54330，用户/库 `codexsymphony_dev` |
| 测试数据库 | 原 `docker-compose.yml`，端口 54329，用户/库 `codexsymphony_test`，tmpfs，可丢弃 |
| 未来工作区与证据 | 预留被 Git 忽略的 `.local-data/workspaces/`、`.local-data/evidence/`；后续 Run 任务实现时接入路径配置，目前 API 不读写这些目录 |
| 构建产物 | 仓库内 `target/`；不能作为工作区、证据或数据库的持久存储 |

需要预留目录时执行 `mkdir -p .local-data/workspaces .local-data/evidence && chmod 700 .local-data`。
若仓库检出本身会被控制器清理，内部试用须将未来持久目录配置到受控的外部存储；
本项不假装已有 Run 路径管理、保全或恢复功能。

`docker compose --env-file .env -f docker-compose.dev.yml up -d --force-recreate --wait postgres`
以及 `down` 后再 `up` 均保留开发卷。`down -v` 会删除开发数据，不能作为普通重启命令。
PostgreSQL 只在空卷第一次启动时使用 POSTGRES_* 初始化；修改 .env 不会更改已有角色密码。
升级前保存数据库和恢复资料；命名卷持久化不等于已验证备份/灾难恢复。

### 隔离测试数据库

```sh
docker compose -f docker-compose.yml up -d --wait postgres
export TEST_DATABASE_URL=postgres://codexsymphony_test:codexsymphony_test@127.0.0.1:54329/codexsymphony_test
cargo test --workspace --locked
```

测试只读取 TEST_DATABASE_URL，绝不从 DATABASE_URL 回退。不要把开发或真实用户库赋给
TEST_DATABASE_URL。原测试 compose 保持 tmpfs；重建容器即丢弃数据。
需要运行 E2E 时，给单独 API 进程设置 `DATABASE_URL="$TEST_DATABASE_URL"`、
`WEB_ORIGIN=http://127.0.0.1:4300`，不要复用持久化内部试用进程。

## Localhost 请求与 CSRF 契约

所有 API 路由最后统一通过 `RequestPolicy::protect` 包装（见 `apps/server/src/security.rs`）。
合法 Host 为**实际监听 IP:端口**；其他 Host、缺失/重复 Host、矛盾的 absolute-form URI 拒绝为 403。
存在 Origin 时只接受 WEB_ORIGIN 或实际 API origin 的精确值；无 Origin 的 GET/HEAD 可供健康探测。
`Forwarded` / `X-Forwarded-*` 不参与授权，无远程代理信任或 CORS 放行。

除 GET/HEAD/OPTIONS 外的请求还必须同时带：

- 合法且唯一的 `Origin`；缺失、`null`、不同 scheme/host/port 均拒绝。
- 唯一的 `X-CodexSymphony-CSRF: 1`；缺失、错误或重复均拒绝。

这是 [OWASP 的自定义请求头 API CSRF 模式](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#employing-custom-request-headers-for-ajaxapi)，
利用浏览器不允许普通跨站表单设置此头且跨源脚本需要 CORS 预检的约束。常量不是秘密 token，
也不是身份认证。即使提供正确头，非预期 Origin 仍拒绝；预检不返回 CORS 授权头。
本机非浏览器进程可伪造头，0a 仅限本人可信主机/代码，不能据此开放远端。

#13 必须将真实写路由放在同一中间件内，Angular 写请求添加上述头，并通过开发代理保持 Origin；
补真实浏览器/数据库的成功、恶意来源、缺头/错头负例，验证拒绝请求无业务副作用。
不得把 GET/HEAD/OPTIONS 实现为写操作。当前只有健康 GET，专用写路由仅存在于测试，
没有业务写接口集成验收。用户名/密码登录仍按 M2，Cloudflare Access 不是产品认证。

## 验证

根目录：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
TEST_DATABASE_URL=postgres://codexsymphony_test:codexsymphony_test@127.0.0.1:54329/codexsymphony_test cargo test --workspace --locked
```

前端目录：

```sh
npm run lint
npm test -- --watch=false
npm run build
npx playwright install chromium
npm run test:e2e
```

E2E 要求 API 与数据库运行；Playwright 自行启动并关闭 4300 端口的前端。
2026-09-15 验证：5 项后端集成测试、6 项前端单元测试通过；桌面和手机各验证
真实数据库健康状态、网络故障后重试，共 4 项 E2E 通过，包含 axe WCAG AA 检查。
格式、Clippy、前端 lint 与 production build 通过。
同日通过 `python3 tools/gate.py verify --profile ci --all` 复核：秘密扫描、架构检查
和全部 7 项执行步骤 PASS；质量判定因缺少可信 ci-state.json 阻断，最终退出码为 1。

## TypeScript CRAP 独立插件（早期 rc.2 历史验收）

实现位于 `~/Harness-Gate-ts-crap/tools/quality/typescript-risk`，分支
`feat/typescript-crap`，提交 `819f88b`。它是 Harness-Gate 的独立 Node collector，
Core 仍负责阈值与最终判定。候选包 0.1.0-rc.2 已安装到本机，入口
`~/.local/bin/harness-gate-typescript-collector`。

15 项插件测试与 4 项已发布 Core 0.4.5 验收通过：完整覆盖的 CC=10 通过，
CC=11 和覆盖不足的 CC=10 失败，原始证据篡改被拒绝。
安装包 SHA-256：`609a87af608149324938a3f9fd608b14b8718fc47bfe067c6bdae6ff80ee4b8d`。
插件包、依赖锁及本地验收记录保存在上述分支；尚未发布正式版本。

前端目录可重复执行 Angular 原始 TypeScript 插桩探针：

```sh
HARNESS_GATE_TYPESCRIPT_PLUGIN="$HOME/.local/share/harness-gate/typescript/0.1.0-rc.2/node_modules/@harness-gate/typescript-collector" node tools/probe-typescript-risk.cjs
```

脚本在临时副本中先插桩再经 Angular 编译运行真实单元测试，保留原始源码、
零计数基线、测试计数、合并覆盖率、插件测量与配置摘要。合并采用每个计数的最大值，
只用于判定是否覆盖，不宣称精确执行次数。当前排除项是列明的 `.spec.ts` 测试文件，
不是由该命名规则自动授权生产门禁排除。当前两个显式函数的 CC=1、CRAP=1，均已覆盖。

探针之后可运行 `tools/frontend_host_acceptance.py`；实际签名采集与前端策略判定已经 PASS，
并拒绝 5 类篡改／过期／重放输入，见 [rc.2 验收记录](quality/frontend-rc2/README.md)。
上述 rc.2 是早期单插件验收。当前 TypeScript 候选版为 rc.4，准确归档摘要以
`tools/gate-plugins/` 清单为准；API 合约、完整多 collector 组合及宿主签名输入已完整通过。
项目 CRAP 上限为 10；本机完整隔离门禁入口见下方。
远端双检查、main 保护与 Symphony Issue→PR→CI→自动合并关闭已实际验收，
见 [远端环境记录](quality/remote-environment/README.md)。该结果不代表 0a 业务验收。
详见 [门禁说明](../.harness-gate/QUALITY.md)。


## Rust 源码 CRAP 独立插件

本机 `harness-gate-rust-source-collector` 为候选 0.1.0-rc.1，独立于 MIR collector。
项目选择源码分支复杂度，CRAP 仍 ≤10；MIR 输出仅作诊断。
实现位于 `~/Harness-Gate-rust-source/tools/quality/rust-source-risk`，提交 `337c363`。

真实源码采集、Core 签名验收与精确指标见
[Rust 源码验收记录](quality/rust-source-rc1/README.md)。10 个函数全部达标，
最大 CC=7、CRAP 约 7.0223；未执行的 async 不会因创建了 Future 而算作覆盖。

启动测试新增真实子进程启动／请求／SIGINT 退出、缺失 DATABASE_URL、无效监听地址。
`BIND_ADDRESS` 可配置监听地址，默认 `127.0.0.1:3081`；测试用 `127.0.0.1:0`
分配临时端口。日志在实际监听成功后才宣布就绪。

候选插件和回放输入保留在 `~/.local/share/codexsymphony/gate-acceptance/`。
本机受信宿主现已安装，不把历史临时验收密钥或请求当作新运行输入。


## 完整本机门禁

```sh
python3 tools/install_gate_plugins.py
~/.local/share/codexsymphony/gate-host/run --repository ~/CodexSymphony
```

安装器按归档 SHA-256 验证三个独立插件；宿主固定在仓库外的版本目录，
以隔离源码副本执行全套检查并保留证据。新运行不自动批准策略或工具变更。
需要更新宿主时，由操作者审查后运行 `tools/install_gate_host.py`；不要在 PR 内调用。

API 响应类型由合约生成，修改合约后执行：

```sh
harness-gate-http-contract-collector generate api/openapi.json HealthResponse web/angular/src/app/health-response.ts
cd web/angular
./node_modules/.bin/prettier --write src/app/health-response.ts
```

`api/baseline.json` 是初始合约的受审副本；宿主使用仓库外固定摘要的基线，
普通 PR 修改该文件不会自动重置比较基准。

完整本地 PASS 与验收范围见 [完整验收记录](quality/complete-local/README.md)。
旧 rc.2／Rust rc.1 的单组件目录是历史记录，以完整记录和当前插件清单为准。

执行边界和普通 Runtime 测试要求见 [可信开发环境](trusted-development.md)。

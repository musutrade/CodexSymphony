# 本机开发与验证

当前骨架：Rust 1.97.1，Axum 0.8，SQLx 0.8，PostgreSQL 16；Node 24.18.0，
Angular 22，Material 22，TypeScript 6.0.2。精确依赖见 Cargo.lock 与前端 package-lock.json。

## 启动

在仓库根目录启动临时数据库和 API：

```sh
docker compose up -d --wait postgres
export DATABASE_URL=postgres://codexsymphony_test:codexsymphony_test@127.0.0.1:54329/codexsymphony_test
cargo run --locked -p codexsymphony-server
```

另一终端运行 `cd web/angular && npm ci && npm start`，打开 localhost:4200。
Angular 代理 `/api/**` 至 localhost:3081；健康接口实际执行数据库查询。
数据库数据在 tmpfs 中，仅供开发测试；`docker compose down` 后丢弃。
业务表尚未建立，`migrations/` 是后续 SQLx migration 入口。

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

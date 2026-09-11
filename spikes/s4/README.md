# S4 spike — Harness-Gate 在本项目上的接入可行性、耗时与拦截能力

日期：2026-09-10　　harness-gate **0.3.7**（从 `~/Harness-Gate/tools/harness-gate` 源码构建，工作区版本 0.3.7）
结论已写回综合方案 23.S、12.6。注意：本机 `cargo install` 装的 `~/.cargo/bin/harness-gate` 是 **0.1.0**，
与源码仓库的 0.3.7 不是同一版本，spike 用的一律是源码构建产物。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | 四项命令能否在本项目跑通 | 能。`config check` 0.05s、`doctor --strict --json` 0.20s、`hook` 0.16～0.51s、`verify --all` 0.36～0.38s（项目当前几乎无代码，这是下限基线）。全部以退出码 0 表示通过。 |
| 2 | `hook` 读哪份配置 | **读暂存区（index），不读工作区。** `.harness-gate/` 未 `git add` 时报 `E1000: read staged workflow configuration .harness-gate/flow.toml: No such file or directory`。把配置物化到 `$TMPDIR/harness-gate-staged-<pid>-<ts>/` 下再校验。这正是 12.6.1 要的"受信快照"语义：Agent 改工作区配置不影响 `hook` 看到的规则。**代价**：配置从未暂存时错误信息误导（看起来像配置缺失，实际是"未暂存"）；平台必须在每次 `hook` 前确保配置已暂存，否则门禁静默失效。 |
| 3 | 拦截能力（反例） | ① 真实形态的 AWS key（`AKIAQ7XZ2MN4PLK9RTUV`）→ secret scan FAIL，`TEST_SUMMARY: FAIL`，退出码非 0；② 暂存含行尾空白的文件 → `staged Git whitespace check ... FAIL`。**误报控制**：`AKIAIOSFODNN7EXAMPLE` 被占位符规则正确豁免（`example` marker），不是漏报——说明 API key 测试字样会被放行，规则设计合理。 |
| 4 | 失败码 | 结构化：`test_result.json` 里每个 step 带 `failure_code`（如 `SECRET_SCAN_FAILURE`）、`passed`、`duration_ms`、`log` 路径。可直接映射方案 12.6.4 的 `gate_failure_code`。 |
| 5 | 执行证据 | 每次 invocation 写 `invocations/<id>/test_result.json`，字段含 `executor_version`、`input_mode`（staged/all）、`source_identity`（`git-tree:<sha>` 或 `working-tree:<sha>`）、`configuration_digest`（`sha256:...`）、`execution_root`。**`configuration_digest` + `source_identity` 就是方案要的"验证策略身份"与"被测源身份"分离**，可直接作为 `trusted_gate_policy_revision` 的指针。 |
| 6 | 配置形态 | `flow.toml` schema v2，内置 preset：`generic` / `rust-api` / `angular-only` / `angular-rust-postgres`。`init` 会生成 `flow.toml` + `audit.toml` + `secrets.toml` + `.gitignore`（`reports/` 默认忽略）。本项目当前只有文档与 spike 脚本，用 `generic`。 |

## 对方案的修正

- **12.6**：锁定版本改成 **0.3.7**（与源码仓库一致），并把"本机 PATH 上可能是旧版本"写成部署检查项——
  平台必须记录实际执行二进制的摘要，不能只记版本号。
- **12.6.1**：补一条硬性要求——**每次 `hook` 前确保 `.harness-gate/` 已暂存**，否则 `hook` 直接报错
  （fail-closed，不会误判为 PASS），但这属于"看得见的失败"还是"静默失效"取决于平台是否检查退出码；
  方案要求平台检查退出码与 `TEST_SUMMARY`。
- **12.6.1**：`configuration_digest` / `source_identity` 的字段名按 0.3.7 实测固定下来，
  替换方案里自造的 `harness_gate_config_digest` 等字段名。
- **12.6.4**：失败码用工具原生的 `failure_code`（如 `SECRET_SCAN_FAILURE`），不要另立一套。

## 复现

```bash
cd ~/Harness-Gate/tools/harness-gate && cargo build --release   # target 在 ~/cargo-target/release
HG=~/cargo-target/release/harness-gate
cd /path/to/repo && $HG init --preset generic && git add .harness-gate/
$HG --project-root . config check
$HG --project-root . doctor --strict --json
$HG --project-root . hook
$HG --project-root . verify --all
```

## 未验证

- **削弱但 schema 合法的暂存配置能否通过 `hook`**（这是 12.6.1 "Agent 不能自证通过"的核心）：
  脚本尝试标注这一点时被本会话的权限分类器拦下，未完成。预期：`hook` 会用暂存配置执行，
  因此**能**通过——门禁的防篡改不来自 harness-gate 本身，而来自平台用独立的受信 policy revision
  比对 `configuration_digest`。这一点必须在 Phase 0c 实现时补测。
- 真实代码量下的耗时（当前仓库几乎为空，0.36s 无参考价值）；`rust-api` preset 在真实 Rust 项目上的
  fmt/clippy/test 耗时。
- `reports/leases` 的并发语义（多 Run 同时 verify）。
- 本 spike 生成的 `.harness-gate/` 是用 `generic` preset 的临时配置，未提交；正式配置应在 Phase 0a
  选定 preset 后一次性建立。

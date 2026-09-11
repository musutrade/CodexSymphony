# S5 spike — Codex 受限网络模式：白名单配置位置、放行与拒绝语义

日期：2026-09-10 初测，**2026-09-11 补测完成**　　codex-cli 0.153.4　　Linux / landlock + bwrap
结论已写回综合方案 23.S、16.9、8.1、22.4、28。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | 机制是否存在 | 存在。`features.network_proxy = true` 后，沙箱任务获得本地 CONNECT 代理（`http_proxy` / `https_proxy` 指向 `127.0.0.1` 随机端口）。开关也可由 managed requirements 直接驱动（见第 4 条，`enabled = true` 时无需显式开 feature）。 |
| 2 | 拒绝语义 | 非白名单域名在代理层被拒：`curl: (7) CONNECT tunnel failed, response 403` + 响应体含 `blocked by policy`。**不产生审批请求**，`approvalPolicy = "never"` 下 turn 正常结束，Agent 拿到明确错误文本继续。 |
| 3 | 绕过路径 | `curl --noproxy '*'` 打裸 IP → 超时（沙箱网络隔离）；SOCKS5 UDP / 非 HTTPS TCP 被禁。 |
| 4 | **配置位置（本轮核心）** | allowlist 只能写在**系统级 `/etc/codex/requirements.toml`** 的 **`[experimental_network]`** 表下（需 root）。源码依据：`ConfigRequirementsToml` 里该字段是 `#[serde(rename = "experimental_network")] pub network`；`system_requirements_toml_file()` 返回 `/etc/codex/requirements.toml`。 |
| 5 | 错误的位置（都实测失败） | ① `/etc/codex/requirements.toml` 里写 `[network]` → 表名不匹配，`configRequirements/read` 返回 `null`；② `$CODEX_HOME/requirements.toml` → 不读；③ 用户 `config.toml` 的 `[network] domains` / `[network] allowed_domains` / `[experimental_network] domains` → 构建了代理但白名单为空，**所有域名全拒**。 |
| 6 | 放行验证 | 写入 `domains = { "crates.io" = "allow", "index.crates.io" = "allow" }` 后：`crates.io` → 200，`index.crates.io` → 200；未列入的 `example.com`、`api.github.com`、`sub.crates.io` → `blocked by policy`。 |
| 7 | 生效范围 | **全局**，影响同机所有 codex 进程；不是 per-Run、也不是 per-session。`configRequirements/read` 可回读确认。 |
| 8 | 排查陷阱 | 上游代理本身可能对目标返回 403（本环境 `static.crates.io` 经用户代理返回 403，与沙箱无关）。判断"是否被沙箱拒绝"要看响应体是否含 `blocked by policy`，**不能只看状态码**。 |

## 对方案的修正

- **16.9**：配置位置写实为 `/etc/codex/requirements.toml` 的 `[experimental_network]`，
  并新增"平台必须把该文件当独占受信资源管理"的实现约束：
  受信控制面写、Agent 不可写、每 Run 前写入并记摘要、Run 后收敛；
  **同机不能并行跑网络集合不同的 Run**——这与 6.3 的并发上限是不同维度的调度约束，
  必须在 Phase 0 的调度条件里显式表达；多 Worker 时按网络集合做主机亲和性分组。
- **8.1**：沙箱默认值里的 `network` 一行指向 16.9 的机制。
- **22.4**：新增四条网络用例（白名单命中、未命中、绕过、越界只生成修订建议）。
- **28**：新增 `network_domain_not_allowed`（blocked，不消耗修复预算）。
- **0.8**：Policy 开关新增 `network_defaults` 与 `allow_requirement_network_scope`。

## 复现

需要 root 写系统级文件；测试期间会影响同机其他 codex 进程。

```bash
sudo mkdir -p /etc/codex
sudo tee /etc/codex/requirements.toml > /dev/null <<'EOF'
[experimental_network]
enabled = true
allow_upstream_proxy = true
domains = { "crates.io" = "allow", "index.crates.io" = "allow" }
EOF
# 用 app-server 的 configRequirements/read 确认已加载，再跑真实 turn 见 README 表格
sudo rm -rf /etc/codex        # 测试后清理
```

## 未验证

- `managed_allowed_domains_only = true` 与用户级 allowlist 的叠加行为。
- deny 覆盖 allow 的实测（配置文件里同时给 `"httpbin.org" = "deny"`，本轮未跑到该用例）。
- 子域是否必须逐条列出：`sub.crates.io` 在 managed 模式下报 `local/private network addresses are blocked`，
  且该名字在本机无真实解析，无法干净区分"未匹配白名单"与"解析失败"，需换一个真实存在的子域复测。
- 白名单是否区分端口（`https://host:8443`）。
- 该文件在 Run 中途被改写时的行为（是否热生效）——**安全上必须假设会热生效**，
  因此平台收敛该文件时必须先确认无活跃 Run。

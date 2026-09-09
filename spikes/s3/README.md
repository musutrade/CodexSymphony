# S3 spike — Cloudflare Access JWT 在 Axum 中的验证、Tunnel 源站隔离、会话撤销

日期：2026-09-09　　Rust 1.97 / axum 0.8 / jsonwebtoken 11 / reqwest 0.12(rustls)
team `higoalzm.cloudflareaccess.com`　　hostname `factory.aglmud.org`　　既有 tunnel（用户级 systemd `cloudflared-codex-gate`）
结论已写回综合方案 23.S、17.4、21.4。

## 结论

| # | 问题 | 结论 |
|---|---|---|
| 1 | Axum 侧验签选型 | `jsonwebtoken 11` + `DecodingKey::from_jwk` + `Validation{set_issuer, set_audience, set_required_spec_claims}` 可直接用。**必须显式开 feature `rust_crypto`（或 `aws_lc_rs`），否则编译通过、首次验签时 panic** "Could not automatically determine the process-level CryptoProvider"。panic 发生在 tokio worker，进程不死但该请求连接被断（curl 得到 000）。 |
| 2 | JWKS | `https://<team>/cdn-cgi/access/certs` 返回 2 把 RS256 key（`keys[]` 带 `kid`）+ `public_certs[]`。按 kid 缓存，未知 kid 时限频重取一次即可。 |
| 3 | 真实 JWT 的 claims | `aud: [<AUD>]`（数组）、`iss: https://<team>`、`sub`（**稳定的身份 ID，电脑与手机相同**）、`email`、`exp/iat/nbf`、`identity_nonce`（**每次登录不同**）、`policy_id`、`type: "app"`、`country`、`h_INTERNAL_DO_NOT_USE`（hostname）。`Cf-Access-Authenticated-User-Email` header 与 claim 一致，但不单独可信。 |
| 4 | 本机负面用例（直连 127.0.0.1:8790） | 无 header → 401 `missing_assertion`；仅邮箱 header 伪造 → 401；垃圾 JWT → 401 `unknown_kid`；**伪造签名 + 真实 kid → 401 `InvalidSignature`**；`alg=none` → 401 `bad_header`（库层拒绝反序列化）；`alg=HS256` 混淆 → 401 `bad_alg`（显式白名单 RS256）；过期 / 错 aud / 错 iss（均伪造签名）→ 401。**全部拒绝，无一放行。** |
| 5 | 真实登录 | 电脑与手机经 Access One-time PIN 登录后均 200，`via_cloudflare: true`（有 `cf-connecting-ip`）。 |
| 6 | 源站隔离 | 服务绑定 `127.0.0.1:8790`；从本机 LAN 地址 `192.168.0.34:8790` 访问 rc=7 不可达。tunnel 以用户级 systemd 运行，`ProtectHome=read-only`、token 文件 `ReadOnlyPaths`。 |
| 7 | 会话撤销 | Access 应用 **Revoke existing tokens** 后，手机与电脑刷新均被踢回登录页。撤销在边缘生效；**源站看到的 JWT 是无状态的**，已签发 JWT 在 `exp` 前直接打源站仍会验签通过（未实测重放，逻辑上必然）。因此"源站仅经 tunnel 可达"是撤销生效的前提，不是可选加固。 |
| 8 | 会话时长 | 本次应用配置实际签发 `exp - iat = 43200s = 12h`。方案 17.4 建议 8h；考虑到第 7 条，不应更长。 |
| 9 | 302 行为 | 未登录访问 `https://factory.aglmud.org/...` 边缘直接 302 到 `https://<team>/cdn-cgi/access/login/<host>?kid=<AUD>`，请求不到源站。 |

## 对方案的修正

- **17.4**：JWT 校验的具体规则定为：`alg` 白名单仅 RS256；`iss == https://<team>`（部署时固定）；`aud` 数组包含固定 AUD；`exp/nbf` 带 ≤30s leeway；`sub` 作为平台 `owner_id` 的绑定值（不是 email）；`email` 与 Policy 白名单二次比对；`type == "app"` 可选校验。`Cf-Access-Authenticated-User-Email` 只用于显示。
- **17.4 撤销**：明确写出"边缘撤销不改变源站对既有 JWT 的判定"，源站不可绕过 tunnel 是撤销语义成立的前提；若将来源站需要在 LAN 上监听（另一台 cloudflared），必须在源站按 `identity_nonce` 维护短期撤销名单，或缩短会话到 ≤1h。
- **21.4**：部署验收第 3 条（伪造头 / 错 AUD / 直连源站均失败）已由本 spike 覆盖，可直接复用 `spikes/s3` 的用例；新增一条"Revoke existing tokens 后刷新被踢回登录页"。
- **21.4 / 0b**：复用既有 tunnel 时用 Public Hostname 追加一条路由到独立端口即可（本次 8790），不需要第二个连接器；但要注意端口冲突——原计划的 8787 已被同机其他服务占用。
- **20 Rust workspace**：`jsonwebtoken = { version = "11", features = ["rust_crypto"] }`；`reqwest` 用 `rustls-tls` 且 `default-features = false`。
- **19.3**：Access 拒绝（`access_unauthorized`）按 `error` 子码计数：`missing_assertion / unknown_kid / bad_alg / invalid_token / email_not_allowed`，便于区分误配与攻击。

## 复现

```bash
cd spikes/s3
cp run.env.example run.env && $EDITOR run.env       # team / AUD / email
cargo build --release
set -a; . ./run.env; set +a
setsid nohup "$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/s3_access_jwt" > s3.log 2>&1 < /dev/null &
# 负面用例：见本 README 第 4 条，用 curl + pyjwt 伪造即可
# 正面用例：Public Hostname 指到 localhost:8790，浏览器登录 /api/whoami
```

不要把 `run.env`、`s3.log`（含邮箱与 sub）提交进仓库；`.gitignore` 已排除。

## 未验证

- 已签发 JWT 在边缘撤销后直接重放到源站（预期验签仍通过；未做，因为需要导出用户 JWT）。
- 会话自然过期（12h）时的边缘行为与 `exp` 是否严格一致。
- Access service token（机器身份）路径——方案 17.6 留作后续。
- 多 IdP（非 One-time PIN）下 `sub` 的稳定性；本次仅 OTP。
- JWKS 轮换：`keys[]` 有两把 key，轮换期间旧 kid 的请求是否仍能命中，需等一次真实轮换。

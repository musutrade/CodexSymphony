# GH-76 同版本 M2 集成记录

验收日期：2026-09-21。范围与逐条 AAuth/B 状态见 [M2 交付矩阵](../../m2-delivery.md)。[可读机器记录](acceptance.json) 保存实际结果、进程身份和原始证据 SHA-256；[环境与迁移](environment.json) 保存 UID/namespace、PG16 版本及实际已应用的 24 条迁移 checksum。

产品源码为完整 main `86f19bacad6c36d025719a61ab9cc46c55e01c77`，包含 #71–#75 最终合并。255 个产品/前端/迁移/部署文件无差异，完整清单在 `artifacts/gh76/product-source.json`。所有本次服务集成使用重新构建的同一二进制：

```text
874d626f9f0f355b22a69452566fff69e5c817522f4874976daf504461ac78ca
```

本 PR 只修改验收工具和文档；`artifacts/gh76/validated-source.json` 记录最终完整源码文件摘要、模式及本地 Git tree，发布后核对远端 tree 完全一致。实际 GitHub PR head 由发布后读回的 `artifacts/gh76/publication.json` 记录，不用本地 HEAD 冒充发布 SHA。

| 验证 | 本次结果 | 原始证据 |
|---|---|---|
| 完整 Rust workspace，含真实 pinned Runtime | passed | `artifacts/gh76/cargo-test-clean.log` |
| Rust build / fmt / 全目标全 features Clippy / check | passed | `build-current.log`、`fmt.log`、`clippy-all-features.log`、`check.log`（均在 `artifacts/gh76/`） |
| Angular build / lint / 单测 | passed，44 项单测 | `artifacts/gh76/frontend-*.log` |
| Desktop Chrome / Pixel 7，真实账号与 HTTPS | passed，28 项 | `artifacts/gh76/browser.log`、`browser-results/` |
| Nginx 模板、证书校验、Origin/代理/Access 负例 | passed | `artifacts/gh76/https-final.log`、`artifacts/gh72/https.json` |
| 到期/撤销/改密/重置和双维限流跨重启 | passed，4 次实际重启 | `artifacts/gh76/auth-session-final.log`、`artifacts/gh71-resume/auth-process.json` |
| API provider 与实际 Angular 消费端合约 | passed，97 个变体 | `artifacts/gh76/auth-contract.log`、`artifacts/gh71-product-contract/` |
| 隔夜问题 → 原工作恢复 → 暂停重启 → 恢复取消 | passed，3 个服务实例、3 个 Run | `artifacts/gh73/recovery/result.json` 及该目录的浏览器/进程日志 |
| Bark 接收器/去重/失败/只读权限/网络隔离 | passed，8 项 | `artifacts/gh76/bark.log` |
| coding/validation 私密文件否定与 supervisor | passed，4 项 | `artifacts/gh76/executor.log` |
| 独立 verifier namespace/策略不可写/重放拒绝 | passed，3 项 | `artifacts/gh76/verifier-boundary.log` |
| 真实 PG16 加密备份、mTLS、隔离恢复、故障路径 | passed，3 项 | `artifacts/gh76/recovery-final.log`、`artifacts/gh75/recovery-acceptance.json` |
| Gate config / architecture audit / secrets | passed | `artifacts/gh76/gate-config.log`、`audit-final.log`、`secrets-final.json` |
| 最终 PR head 的 Harness-Gate / Trusted Harness-Gate | pending：独立宿主 | 当前本地记录不是正式签名质量结果 |

隔夜结果逐阶段核对数据库和工作文件：最初 1 个 Run；超时后 1 份保存；手机回答后第 2 个 Run 实际读取原工作；暂停/重启仍为 2 个 Run，未自动续跑；明确恢复产生第 3 个 Run，携带原问题回答；取消后 `Cancelled`、3 份保存、所有 Run 静止、执行槽释放，原授权和预算上限不变。未把取消写成 Done。模型协议对端与 GitHub 能力观察为明确标注的合成 fixture；真实 GitHub 写与模型服务不在本次测试范围。

恢复比较 73 项表/序列/通知投影事实，涵盖父子草稿/依赖/AC、组授权、已用+预留、暂停/取消、问题/保存阶段、撤销/过期 session、限流及 Bark 台账；同时比对 Git 暂存/未暂存/未跟踪文件。恢复库两次只读启动，业务写返回 404/405，普通启动在迁移前拒绝；这不等于生产切换或恢复后自动执行。

浏览器还检查 axe、无页面横向溢出、未提交草稿保留且不自动执行、共享数据和离线后待办。已复核移动整组评审截图的标题、冲突提示、依赖和授权状态；截图仅辅助界面验收，不承担认证/隔离安全证明。

本次发现的两处证据目录缺失已在原验收入口修正。最初 Rust 全量运行因长 `TMPDIR` 超过 Unix socket 路径上限失败，保留 `artifacts/gh76/cargo-test.log`；改用短路径直接重跑又遇到中断留下的 `storage_guard`，预算预留安全拒绝，见 `cargo-test-final.log` 与 `failed-fixture-storage.json`。通过提供的 helper 仅重建 disposable test fixture，回执 `test-fixture-recreate.json` 保存前后容器身份；再使用映射到工作区的短 `/tmp` 完整重跑，不改断言或产品策略。原恢复失败日志为 `artifacts/gh76/recovery.log`。构建的 552.11 kB 初始包仍超过 500 kB 警告阈值，构建成功，阈值未调整。

原始证据通过 `.symphony-evidence.json` 的 canonical 路径与 SHA-256 交接；宿主是否复制以 after_run 回执为准。没有生产操作、真实 Bark 发送或真实离机备份。实际公网/实体双设备、真实 Access 身份头、通知权限/出口、离机 custody/恢复与部署 grant 仍为 blocked；最后步骤及回退条件见 [上线清单](../../m2-delivery.md)。B01/B04–06 和 B08 CI/自动合并部分仍 out-of-M2。

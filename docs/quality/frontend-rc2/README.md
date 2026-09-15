# Angular 原始 TypeScript 签名门禁验收

2026-09-15，本机独立安装的 TypeScript collector 0.1.0-rc.2 经 Core 0.4.5
完成签名采集与质量判定，结果 PASS。范围是当前 Angular 骨架的原始 TypeScript，
不是全项目、生产部署或模板分支覆盖率认证。

- 15 项插件单元测试、4 项协议／Core 测试、6 项 Angular 单元测试通过。
- 5 个可执行 TypeScript 文件逐文件行覆盖率达到 80%；两个函数的行／函数覆盖率
  达标，CRAP 均为 1，上限仍为 10。
- `health-response.ts` 经 AST 确认为纯接口声明；保留其文件身份和非数值证据，
  不给已擦除的类型代码虚构覆盖率，也不以“零计数”作为排除依据。
- 旧上下文、超过 Core 30 秒时钟容差的过期签名、签名篡改、重放、原始证据篡改
  共 5 项反例均被拒绝。
- 宿主在测试结束后创建隔离于测量目录之外的临时密钥，签名后立即删除私钥。
  这是本地可信输入演练；同 UID 环境不构成生产隔离或跨信任边界证明。

复现：在前端目录运行 `tools/probe-typescript-risk.cjs`，然后在仓库根目录执行：

```sh
python3 tools/frontend_host_acceptance.py \
  --bundle /探针输出目录/collector-bundle.json \
  --plugin "$HOME/.local/share/harness-gate/typescript/0.1.0-rc.2/node_modules/@harness-gate/typescript-collector"
```

每轮使用新的探针目录。宿主程序只接受带本地探针标记的副本，不覆盖项目正式配置。
测试负例完成后恢复原始证据，保留正向报告、拒绝日志及状态输入。

`acceptance.json` 固定安装包、测量系列与 `evidence.tar.gz` 的摘要。归档包含实际
源码、测试、模板、原始计数、签名请求、公开信任表、Core 报告和拒绝日志；无私钥。
签名具有短有效期且绑定原路径，归档用于审查，不是可重放的生产运行输入。

正式 `ci` 配置仍保留全项目 required 策略：Rust、API 合约及宿主部署未验收前，
不得拿这一份前端 PASS 替换完整门禁结果。

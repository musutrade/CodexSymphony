# CI 性能诊断与优化

本文说明当前可信宿主的构建缓存、运行时间记录，以及进一步优化时需要核查的边界。

## 验证流程与变更边界

GitHub Actions 等待可信宿主为**同一提交、同一次 Actions 尝试**发布的 `Trusted Harness-Gate` check。宿主执行隔离门禁并发布结果；Actions 不执行完整门禁。

宿主在运行门禁前，核对当前提交中的受保护文件、可信入口和策略文件是否属于一套完整的已审核部署。修改这些文件的 PR 若没有对应的宿主审核与部署，会收到 `source does not match a complete reviewed deployment`。这不适用于所有 Git 跟踪文件：普通源码变更可以接受正常验证。PR [#107](https://github.com/musutrade/CodexSymphony/pull/107) 因改动受保护输入而被拒绝，随后关闭，未合并。

当前文档不在宿主的文档快捷验证白名单中。若把它单独提交到 PR，宿主仍会执行完整门禁；这与上述部署匹配检查是两回事。

## 已有的构建缓存

`tools/quality-host/build_cache.py` 管理依赖构建缓存：完整通过的 `main` push 运行尝试发布种子，PR 运行恢复其独立副本。宿主按事件自动决定是否传入 `--publish-cache`，无需在 PR 上手动运行该命令。缓存默认保留 7 天，总容量上限 40 GiB；缓存命中也不保证项目代码、测试或覆盖率采集无需重新执行。

缓存根目录为 `~/.local/share/codexsymphony/gate-host/build-cache/`。每次运行的 `cache-restore.json` 记录是否命中；`main` 运行的 `cache-publish.json` 记录是否发布成功。缓存键包含仓库中的 Cargo 清单、锁文件和 `.cargo` 配置，以及批准的运行时、策略和 Rust/Cargo 版本。详见[现有门禁性能说明](quality/gate-performance.md)。

```bash
run=$(cat ~/.local/share/codexsymphony/gate-host/latest-run)
cat "$run/cache-restore.json"
test ! -f "$run/cache-publish.json" || cat "$run/cache-publish.json"
ls -ld ~/.local/share/codexsymphony/gate-host/build-cache/
```

若未命中，先检查最近一次完整通过的 `main` push 是否发布了缓存，再比较该运行与 PR 的 Cargo 输入和工具链版本。仅更新 PR 所基于的 `main` 不保证缓存命中。

## LLD 的现状

在宿主用户的 `~/.cargo/config.toml` 中设置 LLD，**不会改变当前 CI 门禁**。隔离器为每次运行创建单独的 `cargo-home`，将它挂载为门禁内的 `~/.cargo`，只从宿主挂载 Cargo registry 和可执行文件目录。直接在宿主执行 `cargo build` 也不能证明隔离门禁使用了相同配置。

若要评估 LLD，需要先审核并部署隔离环境中的链接器配置及所需工具，确保缓存键覆盖新增的编译输入，再用相同提交和相同条件的门禁运行对比。修改 `tools/quality-host/*.py`、项目级 `.cargo` 配置等受审核输入，需要遵循完整的宿主审核与部署流程。当前没有 LLD 的门禁实测加速数据。

## Actions 轮询与等待预算

截至 2026-09-23，已合并的 `main` 上 `.github/workflows/quality.yml` 的轮询间隔是 15 秒。缩短至 5 秒最多可让 Actions 早约 10 秒发现一次已完成的宿主检查，不能缩短 Rust 编译或宿主排队时间。PR [#108](https://github.com/musutrade/CodexSymphony/pull/108) 曾单独尝试此改动，也因工作流属于受保护输入而被宿主拒绝，现已关闭。

仓库变量 `GATE_QUEUE_MINUTES`（默认 60，最多 120）和 `GATE_EXECUTION_MINUTES`（默认 45，最多 55）控制等待超时预算，**不控制轮询间隔，也不是性能优化开关**。调整这些变量只应依据实际排队和执行时间。

## 用记录测量耗时

门禁运行目录中的 `timings.jsonl` 记录各阶段的单调时钟耗时，包括 `cache-restore`、`capture-all` 和 `verify`。`capture-all` 内还记录子阶段；不要把父阶段和子阶段相加作为总耗时。Actions 的端到端时间还包括排队与结果轮询，应从对应的 Actions 运行单独读取。

只读分析工具可以汇总最近运行的缓存命中、阶段跨度、后端采集、Cargo 编译与各 Rust 测试程序耗时：

```bash
python3 tools/analyze_gate_performance.py --limit 5
python3 tools/analyze_gate_performance.py ~/.local/share/codexsymphony/gate-host/runs/run-d16822ae40b3
```

输出中的 `other_backend_seconds` 是后端采集总耗时减去 Cargo 报告的编译时间和测试程序耗时，**还不能单独归因为覆盖率导出**；其中也包含 Cargo 启动及测试程序之间的开销。若要精确测量覆盖率合并和导出，需对已批准的 Rust collector 增加计时并按宿主审核流程部署。

例如 `run-d16822ae40b3` 的后端采集耗时 552.14 秒，其中编译 112 秒、测试程序合计 346.97 秒、其余 93.17 秒；最慢的 `github.rs` 测试程序耗时 102.91 秒。因此，应先定位该测试程序内的慢测试，再评估实现或测试夹具的优化，同时保留原有验证范围。

```bash
run=$(cat ~/.local/share/codexsymphony/gate-host/latest-run)
cat "$run/timings.jsonl"
cat "$run/cache-restore.json"
```

现有[性能记录](quality/gate-performance.md)中，PR #96 的 Actions 参考运行耗时 32 分 18 秒，其中后端采集 15 分 23 秒、普通后端测试 8 分 56 秒。另查看本机保留的五次完整通过记录，按首阶段开始到末阶段结束计算：

| 门禁运行 | 缓存命中 | 阶段跨度 |
| --- | --- | --- |
| `run-fc8916002466` | 否 | 19.0 分钟 |
| `run-9fd0ecdf3cc4` | 否 | 16.9 分钟 |
| `run-0283e427d61b` | 是 | 17.1 分钟 |
| `run-5dae02854be2` | 是 | 14.6 分钟 |
| `run-d16822ae40b3` | 是 | 14.1 分钟 |

这些运行的提交和执行条件不同，不能据此宣称缓存或 LLD 带来固定百分比的提升。原先“完整 CI 首次约 12 分钟、有缓存约 3–4 分钟”等数字没有对应的门禁测量依据，现不作为预期结果。

后续优化应分别记录相同提交、相同批准部署下的缓存命中情况、`timings.jsonl` 阶段耗时和 Actions 端到端耗时，再比较变化。

文档快捷验证白名单目前只列出少数说明文件。截至 2026-09-23，最近 40 个已合并 PR 中只有 2 个全部由 Markdown 文件构成；暂不为这份文档扩展受保护的宿主白名单。若以后文档 PR 明显增加，应先审查文件内容是否纯说明，再按完整宿主部署流程更新白名单。

## 相关实现

- [可信宿主部署匹配](../tools/remote-gate/host.py)
- [隔离环境与 Cargo 目录](../tools/quality-host/isolation.py)
- [构建缓存](../tools/quality-host/build_cache.py)
- [阶段计时](../tools/quality-host/timing.py)
- [只读性能分析工具](../tools/analyze_gate_performance.py)
- [完整门禁说明](../.harness-gate/QUALITY.md)

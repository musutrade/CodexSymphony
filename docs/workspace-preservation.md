# 开发工作区证据保全

开发控制器清理终态 Issue 的工作区前，必须完成独立归档。该规则不负责产品
storage_material 的回收，也不改变需求 owner、模型预算或 GitHub 合并事实。

每个任务在交接前写入 `.symphony-evidence.json`：

```json
{
  "schema": "symphony-evidence/v1",
  "files": [
    {"path": "artifacts/result.json", "sha256": "填写该文件的64位小写SHA-256"}
  ]
}
```

清单必须覆盖任务声明的必需原件。确实没有证据的任务可使用空 files，但必须给出
`empty_reason`；缺失文件不能当作没有证据。旧工作区没有清单时保留原目录，由人工
核对其必需原件后补清单，不能自动生成空清单放行。归档不证明验收通过或证据语义正确。

归档器只读取规范化的相对路径和普通文件，拒绝路径穿越、符号链接及工作区内归档。
清单最多 1 MiB、4096 项，文件合计最多 1 GiB；超限保留原目录并报告原因。
它在独立目录临时复制，核验 SHA-256，fsync 后原子发布包含原清单和 receipt 的归档。
重复执行核验当前原件及现有归档；中断没有成功收据，重试不删除原件，不覆盖损坏归档。
归档目录不得提供给编码 Run 写入，本次不增加归档自动淘汰策略。

安装后的命令为：

```sh
python3 /home/gem/.local/share/codexsymphony/symphony/preserve_workspace.py "$PWD" \
  --archive /home/gem/.local/share/codexsymphony/symphony/retained-evidence
```

成功输出 receipt 路径，写入交接的 validation_summary。清理入口独立重跑同一命令，
并启用 `hooks.before_remove_required: true`：缺失钩子、非零退出、超时均阻止删除。
普通删除、记录路径删除及 SSH 删除采用同一约束；失败原因写入控制器日志。
配置重新加载和进程重启后仍有效。默认 false 保留其他部署既有行为。

## 控制器与安装约束

控制器扩展保存在 `tools/symphony/controller-preservation.patch`，基于本机已使用的
Symphony 提交 `588686d`，对应本地提交 `5bc4beed9988f25f0161f6061986d6ca905f8f33`。它不是上游已发布的功能。
在干净的对应源码上应用补丁，按 `elixir/AGENTS.md` 执行 `mise exec -- make all`；
不得只修改工作流就认为旧控制器会阻止删除。

安装器在替换任何活动工作流前，读取宿主
`~/.local/share/codexsymphony/symphony/preservation-controller.json`，核对
`binary_sha256` 与 `~/symphony/elixir/bin/symphony`、`patch_sha256` 与仓库补丁。
该记录由宿主完成上述构建测试后写入；它是宿主部署记录，不是 PR 自批 Gate 的入口。
缺失记录或任一摘要不符时拒绝安装。验证后安装器复制归档器并保留现有标签路由。

受保护工作流和安装器的修改仍按 [远端门禁](remote-gate.md) 重新审定。
部署顺序为：审定受保护输入、精确提交双 Gate、安装、空闲时重启开发控制器、
回读预算和交接台账。不能重置台账以实现升级。

## 验证

```sh
python3 -m unittest discover -s tools/symphony -p 'test_*.py' -v
```

在修复后的 Symphony `elixir/` 中运行真实联动验证：

```sh
mise exec -- mix run --no-start /home/gem/CodexSymphony/tools/symphony/preservation_smoke.exs
```

它使用一次性合成目录，不连接 tracker、不启动模型，验证缺少清单和摘要不符时原件
保留，重启配置存储后修复清单，成功删除工作区后独立归档仍可读取。Python 测试另覆盖
中断、fsync 失败、现有归档损坏、源文件变化及路径攻击。

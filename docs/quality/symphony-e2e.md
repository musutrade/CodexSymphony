# Symphony 开发环境端到端验收（GH-27）

日期：2026-09-15。范围仅为现有 Elixir Symphony 开发控制器的隔离执行、
宿主 API 发布和 CI 交接；本 PR 只新增本文档。

## 基线与准备

- Issue：[GH-27](https://github.com/musutrade/CodexSymphony/issues/27)。
- 准备 PR：[#26](https://github.com/musutrade/CodexSymphony/pull/26)，宿主 API
  回读 `merged=true`，合并 SHA 为 `b9ec6593759907e1c9c254339fb1efe2802084df`。
- `pwd`：`/home/gem/.local/share/codexsymphony/workspaces/GH-27`。
- `git branch --show-current`：`symphony/GH-27`。
- `git rev-parse HEAD`：`b9ec6593759907e1c9c254339fb1efe2802084df`，与远端
  `refs/heads/main` 一致；基线 tree 为 `45fb43c8896f6cd53a859d2135d25235ad2401fa`。
- 初始 `git status --short` 输出为空，无已保存交接文件；远端 Issue 分支返回
  404，同分支 PR 查询为空。保留控制器提供的分支和基线。
- `codex --version`：`codex-cli 0.154.0`，退出码 0，与
  `cat codex-version.lock` 一致。另有 PATH aliases 无法在只读文件系统创建的警告。
- `github_api` 成功读取仓库与 Issue（HTTP 200），确认宿主工具可用。
- 工作区可写，工作区内 `target/` 已存在且可写。本次没有 Rust/Angular 改动，
  不需要本地编译、数据库服务或真实 GitHub/cloud/notification spike。

## 配置检查：首次失败与恢复

首次实际命令：

```sh
python3 tools/gate.py config check
```

退出码 1：

```text
Unrecognized harness-gate version/digest; reviewed Linux amd64 release required
```

只读诊断 `command -v harness-gate` 定位到 `/home/gem/.cargo/bin/harness-gate`；
`harness-gate --version` 为 `harness-gate 0.4.2`。沙箱 PATH 将该目录排在
已挂载的锁定版 0.4.5 之前。此结果说明宿主 hook 的成功不能替代实际沙箱检查。

仅为重试命令优先选择已有锁定二进制，未更改文件、门禁、版本锁或沙箱策略：

```sh
PATH="/home/gem/.local/share/harness-gate/versions/v0.4.5/bin:/home/gem/.local/share/harness-gate/versions/rust-collector-v0.1.0-rc.6/bin:$PATH" python3 tools/gate.py config check
```

退出码 0，实际输出：

```text
Configuration valid: /home/gem/.local/share/codexsymphony/workspaces/GH-27/.harness-gate/flow.toml
Schema version: 2
Components: backend, frontend
Profiles: ci, full, hook
Verification steps: 7
Quality configuration valid: .harness-gate/quality.toml (v1)
```

`tools/gate.py` 在执行前核对两个程序的版本与发行 SHA-256。
本次配置检查已通过；默认 PATH 的优先级问题仍存在，后续运行需要相同显式选择，
或由获授权的环境维护修复。本次不修改 WORKFLOW 或启动器。

## 沙箱检查

以下为实际执行的 Python 检查（退出码 0）。两个敏感路径只检查存在性，
没有打开、读取或输出凭据内容。Git 写入只尝试以排他创建方式创建指定 canary；
若意外成功，仅删除本次创建的 canary，并将验收标为失败。

```sh
python3 - <<'PY'
import errno
import os
from pathlib import Path
workspace = Path.cwd()
print(f'workspace={workspace}')
print(f'workspace_writable={os.access(workspace, os.W_OK)}')
print(f'target_exists={Path("target").is_dir()} target_writable={os.access("target", os.W_OK)}')
git_dir = workspace / '.git'
mounts = []
for line in Path('/proc/self/mountinfo').read_text().splitlines():
    fields = line.split()
    if fields[4] == str(git_dir):
        mounts.append(fields[5])
print(f'git_mount_options={mounts}')
canary = git_dir / 'environment-acceptance-canary'
try:
    fd = os.open(canary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
except OSError as exc:
    print(f'canary_create=FAILED errno={exc.errno} message={exc.strerror}')
    canary_ok = exc.errno in (errno.EROFS, errno.EACCES, errno.EPERM)
else:
    os.close(fd)
    canary.unlink()
    print('canary_create=UNEXPECTED_SUCCESS self_created_canary_removed=true acceptance=FAIL')
    canary_ok = False
hidden_ok = True
for path in (
    '/home/gem/.secrets/my-disposable-bot.2026-09-08.private-key.pem',
    '/home/gem/.local/share/codexsymphony/gate-host/approval.json',
):
    visible = os.path.lexists(path)
    print(f'path={path} exists={str(visible).lower()}')
    hidden_ok = hidden_ok and not visible
readonly = bool(mounts) and all('ro' in options.split(',') for options in mounts)
passed = readonly and canary_ok and hidden_ok
print(f'sandbox_acceptance={"PASS" if passed else "FAIL"}')
raise SystemExit(0 if passed else 1)
PY
```

实际结果：

```text
workspace=/home/gem/.local/share/codexsymphony/workspaces/GH-27
workspace_writable=True
target_exists=True target_writable=True
git_mount_options=['ro,nosuid,nodev,relatime', 'ro,nosuid,nodev,relatime', 'ro,nosuid,nodev,relatime']
canary_create=FAILED errno=30 message=Read-only file system
path=/home/gem/.secrets/my-disposable-bot.2026-09-08.private-key.pem exists=false
path=/home/gem/.local/share/codexsymphony/gate-host/approval.json exists=false
sandbox_acceptance=PASS
```

这些观察绑定当前执行沙箱和上述路径，不推论同 UID 的所有文件均受到读隔离。

## 宿主发布与控制器交接

遵循 [WORKFLOW.lifecycle.md](../../WORKFLOW.lifecycle.md)：Agent 调用注入的
`github_api`，由沙箱外的 Elixir 宿主使用其配置凭据发布 Git blob/tree/commit/ref
和同仓 PR。shell 不使用 `git push`、`gh` 或携带凭据的 curl。
只读 Git 用于核对工作区；`.git` 只读不阻止宿主交付。

发布以确认的基线为 parent，保留 base tree 的其他条目，仅加入本文档（模式
`100644`）。创建 `symphony/GH-27` ref 和指向 main 的 PR；若远端身份意外变化，
先对账，不覆盖分歧。发布后回读 ref、PR head 和递归 tree，逐项核对路径、模式及
blob SHA 与已验证工作区一致。实际 PR 编号与完整远端 head SHA 记录于工作区根目录
不入 Git 的普通文件 `.symphony-handoff.json`，核对成功后才写入并退出。
本地 HEAD 可以仍为基线，不能冒充发布 SHA。

交接状态为 **submitted; CI pending**。独立 App 宿主针对精确 PR head 运行
`python3 tools/gate.py verify --profile ci --all`；GitHub Actions 等待其带 App
身份的结果。完整 required gate 仍适用，CRAP ≤10、覆盖率 ≥80% 及证据要求未改变。
本地配置和隔离检查不能证明完整门禁通过。

控制器后续等待两个 protected checks：`Harness-Gate`（GitHub Actions）与
`Trusted Harness-Gate`（独立 App），核对发布者及精确 SHA，通过 SHA 守卫合并，
核对交付后关闭 GH-27。Agent 不轮询 CI、不合并、不关闭 Issue。
这部分验收在发布时仍待控制器完成，不能预先宣称成功。

## 产品范围边界

本链路只证明开发环境，不替代 #12 的范围核对，也不代表产品业务验收或 0a 完成。
#12 可复用 #26 的工程基线，后续仍按 #12 → #25 依赖顺序实施，0a 验收以综合方案
第 23、24 章为准。Requirement 业务事实、AgentRun 执行事实、GitHub/CI 观察与
身份绑定的验证证据保持独立；不能将 Gate PASS 推导为 Done，或把验证失败改写为
AgentRun 失败。未来 Rust 产品的 0a 使用 custom validation，结束于 Submitted/PR；
不移植本文所述开发控制器交接协议。Cloudflare/Bark 属于 0b，独立 executor UID
与新锁定 Harness-Gate 集成属于 0c。

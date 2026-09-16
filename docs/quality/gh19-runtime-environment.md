# GH-19 Runtime 环境恢复（2026-09-16）

原阻塞在 initialize 之前退出。仅设置 sqlite_home/log_dir 不足以隔离 Codex 的
全部可写运行状态；共享 CODEX_HOME 在 command/exec 内只读。为测试子进程创建
工作区内独立 CODEX_HOME 后，真实 app-server 可以握手并创建 thread。原 stderr
中的 bubblewrap 警告仍可能存在，它不等于 initialize 失败。

## 实际验证

在分配的 GH-19 命令沙箱内执行：

```sh
python3 /opt/symphony-env/runtime_smoke.py
```

- Codex：`codex-cli 0.154.0`；UID `1000`。
- 工作区、进程 cwd、thread cwd：`/home/gem/.local/share/codexsymphony/workspaces/GH-19`。
- 被测工作区 Git HEAD：`cba6a54475209405b64ee6bca0720444cd054be6`（保留原准备阶段基线）。
- 安装探针 SHA256：`26d534c36006e816f3a2e11fd7b15d4e004299362e154166676b081b5af9a525`。
- initialize/initialized、thread/start、动态工具请求/回复、turn/interrupt：PASS。
- `.git` 只读，宿主凭据与 Gate approval 不可见。
- provider 为当前命令网络命名空间内的固定 Responses fixture；真实模型调用 0，
  不复制/链接认证、不扩大网络许可、不关闭沙箱。
- 临时运行状态随退出删除；探针只输出小型 JSON，异常输出限制为末尾 4000 字符。
- 宿主复盘记录：`symphony/gh19-runtime-recovery/runtime-smoke.json`。

## 效力边界

这证明 GH-19 所需的 app-server 协议 smoke 可在现有隔离内执行。它不证明 Rust
产品 Runtime 已实现或通过验收，不证明生产模型联网认证，也不证明更深层的
shell 沙箱能够执行。后续产品测试应调用实际 adapter，并分别验证这些边界。

通用环境模板为现有和新 Issue 安装同一探针；状态目录按测试隔离并清理。
GH-19 恢复时保留原阻塞报告及累计预算，不重置尝试次数。

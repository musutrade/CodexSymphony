# Symphony 上游改进采纳（2026-09-16）

## 已采纳

- 开发宿主 GitHub App HTTP 客户端拒绝全部重定向，调用方收到 HTTP 失败；
  不自动转发认证头、不重放写请求。仅接受同源绝对 API 路径。
- Python 使用真实本地 HTTP 服务覆盖直连、同源与跨源的
  301/302/303/307/308、GET/POST、错误响应及非法路径；另覆盖 HTTPS 降级。
  24 项 remote-gate 测试通过。
- Rust 产品 GitHub 客户端已有禁用重定向策略；本次增加真实 HTTP 回归测试，
  验证各类跳转均无法到达同源或跨源目标。完整编译、测试与覆盖率由本 PR Gate 验证。
- GH-19 验收补充实际子进程凭据隔离：GitHub/GitLab/OAuth 别名、自定义变量、
  合成哨兵、失败日志与正常工具配置；不增加 GitLab 产品功能。

## 本地 Elixir Symphony

将既有 23 个源码、文档和测试文件的定制改动保存在 `local/codexsymphony` 分支，
提交 `bf8cd2b`，再独立采纳上游 #124 为 `7392c57`（保留上游来源）。
根目录的历史二进制和日志没有混入源码提交。没有整体覆盖为官方发行版。

独立检出通过 `make all`：345 tests、0 failures、6 skipped；配置要求的覆盖率
100%，格式、lint/specs 和 Dialyzer 通过。跳过项未冒充真实外部 E2E 验收。
已将经过验证的 escript 原子安装到原启动路径，保留旧文件；运行中的 GH-19
没有重启或热加载。GitLab 修复在下次正常控制器启动后对运行进程生效；当前
GitHub 请求路径原本已拒绝重定向。

复盘摘要与小型日志保留在宿主
`~/.local/share/codexsymphony/upstream-adoption-20260916/`。

## 后续发布约定

产品进入分发阶段时复用同一构建和 smoke 测试流程生成稳定版与预发布；记录
精确源码 SHA 和产物 SHA256。滚动 nightly 标签只作为下载入口，不能替代证据身份。
产物保留须服从现有 TTL、字节预算与保护规则。本次不增加四平台 nightly 构建，
不改变文档快速 CI、串行开发或产品阶段范围。

来源：[认证别名清理 #119](https://github.com/openai/symphony/pull/119)、
[GitLab 跨源认证修复 #124](https://github.com/openai/symphony/pull/124)、
[nightly 发布改进](https://github.com/openai/symphony/commit/be10a1b79df723d6d7612b5651c8522704dafb2e)。

# HTTP 夹具恢复记录

原 blocked.md 是历史检查点，其“必须等待新的 reviewed Runtime 宿主入口”要求已被 PR #49 的通用接口替代。

当前支持项目内 api/capture-fixture.sql，由独立验收在每次新数据库迁移后、请求采集前加载；字段/角色归属由项目夹具维护。SQL 哈希纳入 receipt；所有响应均由实际 API 产生，不伪造结果。接口见 docs/remote-gate.md。

当前夹具已在已安装的独立入口、干净数据库上重复成功：31 条真实观察，问题回答与所属证据读取 HTTP 200，已安装 rc.4 collector 判定 compatible=true/client_drift=false。原始证据在 target/gh22-fixture-recovery/。测试数据不冒充真实 Runtime/A01。

继续原实现：UI/状态/版本接续、错误场景、浏览器测试、完整 lint/build/Rust 测试、逐函数覆盖和 CRAP 尚需收尾。原历史预算保留，不因本次 fixture 恢复启动新实现。

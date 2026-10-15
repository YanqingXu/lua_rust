# v0.15 · Coroutine

本快照让每个 coroutine 拥有独立的暂停帧、寄存器窗口、vararg 和 open upvalue。`yield` 是 VM 内部控制信号；它逐层保存调用帧，`resume` 再从最内层向外恢复，不会重新执行已经产生过副作用的源码。

```powershell
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml -- `
  book/code/v15-coroutine/examples/demo.lua
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --workspace
```

本版提供 Lua 5.1 的 `coroutine.create`、`resume`、`yield`、`status`、`running` 和 `wrap`。暂停状态由 Heap 中的 thread 对象持有强边，因而寄存器、闭包和环状 table 能跨 GC 存活；不可达的暂停线程及其状态会被回收。

当前仍没有标准库 crate、受保护调用 API 或完整 debug hook；教学辅助 GC API 也尚未换成正式的 `collectgarbage` 接口。

## 沿五个问题逐步验证

[执行包](exercises/continuation/README.md) 依次替换全局 scratch、从入口重跑、共享嵌套
owner/yield 通道三个设计；每题只修改一个 hook。固定错误版展示交错状态污染、重复
副作用和返回方向错误，真实 VM 另验收交错 A/B、深调用/多 yield、含 nil 多值及 status。

[roots 包](exercises/roots/README.md) 区分 VM 的 Rc 执行槽与 Heap 的 Value 镜像，再
替换漏暂停镜像、把全部 scheduler/upvalue 索引当根两种设计。真实 Heap 检查对象实际
访问与撤根回收；真实 VM 检查恢复值 46，以及弱槽精确 nil、scheduler count 0。

```powershell
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example continuation_problem_chain
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example coroutine_roots_problem_chain
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml -- book/code/v15-coroutine/examples/resume_once.lua
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_reference --test gc_coroutine_roots_reference

# 初始分别 3 项、2 项失败，完成对应 starter 后必须全部通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_exercises -- --ignored
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test gc_coroutine_roots_exercises -- --ignored
```

默认 workspace 保留原 99 项测试，加 18 项参考验收，合计 **117 项通过、5 项 TODO ignored**。
两个 CLI 不读取 starter，退出 0 只表示固定对照重放完成；练习完成以显式 `--ignored`
为准。模型只覆盖单帧执行/局部 owner 协议或稳定暂停时的 GC 图，不证明全部 VM 路径。

## 独立记录的现有失败入口

```powershell
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml -- book/code/v15-coroutine/examples/active_varargs_gap.lua
```

当前应以非零退出并报 `stale GC handle`：Lua tail-call 参数可能绕过 invoke 临时根，
活动 varargs 又没有独立根枚举。这个已知功能缺口没有在本教学包中修复，也不属于五个
TODO；暂停 varargs 存活的正向验收不能外推到所有活动参数转交。精确输入在上述文件，
后续工程整改可从同一输入接续。

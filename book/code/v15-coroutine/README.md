# v0.15 · Coroutine

本快照让每个 coroutine 拥有独立的暂停帧、寄存器窗口、vararg 和 open upvalue。`yield` 是 VM 内部控制信号；它逐层保存调用帧，`resume` 再从最内层向外恢复，不会重新执行已经产生过副作用的源码。

```powershell
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml -- `
  book/code/v15-coroutine/examples/demo.lua
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --workspace
```

本版提供 Lua 5.1 的 `coroutine.create`、`resume`、`yield`、`status`、`running` 和 `wrap`。暂停状态由 Heap 中的 thread 对象持有强边，因而寄存器、闭包和环状 table 能跨 GC 存活；不可达的暂停线程及其状态会被回收。

当前仍没有标准库 crate、受保护调用 API 或完整 debug hook；教学辅助 GC API 也尚未换成正式的 `collectgarbage` 接口。

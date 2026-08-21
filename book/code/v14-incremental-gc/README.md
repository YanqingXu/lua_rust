# v0.14 · 增量 GC、弱表与 Finalizer

本快照把 v0.13 的 stop-the-world 完整回收拆成 `Pause → Mark → Atomic → Sweep`，每个 VM checkpoint 按预算推进。Table、metatable 和 upvalue 写入统一执行 black-to-white barrier；分配计数达到阈值后自动启动回收。

```powershell
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml -- `
  book/code/v14-incremental-gc/examples/demo.lua
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --workspace
```

弱表读取 metatable 的 `__mode = "k" | "v" | "kv"`；weak key 使用 ephemeron fixpoint。最小 userdata 可通过教学接口 `newuserdata` / `setfinalizer` 注册 Lua finalizer，支持重入、resurrection 和只执行一次后的 second death。

`gc_step`、`gc_phase`、`gc_collect`、`gc_live` 以及 userdata helper 仍是阶段诊断 API。完整标准库、Lua 5.1 userdata/debug API、coroutine 与最终兼容命名尚未实现。

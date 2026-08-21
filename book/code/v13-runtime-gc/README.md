# v0.13 · Runtime 所有权与完整 GC

本快照把 v0.12 的 heap 对象从 `Rc<RefCell<_>>` 迁移到 Runtime 唯一拥有的 arena。Table、closure 和 upvalue 通过不可拥有对象的 `GcRef<T>` 访问；handle 同时保存 heap ID 与单调递增的 object ID。

```powershell
cargo run --manifest-path book/code/v13-runtime-gc/Cargo.toml -- `
  book/code/v13-runtime-gc/examples/demo.lua
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --workspace
```

`gc_collect()` 与 `gc_live()` 是本阶段的教学诊断入口。完整 mark/sweep 从栈帧、global、registry、活跃函数/Proto、open upvalue、primitive metatable 和临时调用值出发；不可达自环可以回收。宿主保留的 Value 应在显式回收前写入 registry。

本版本暂不支持增量 GC、write barrier、自动 checkpoint、weak table、finalizer、userdata 和 coroutine。标准 `collectgarbage` 属于后续标准库版本；当前两个 `gc_*` 名字不会作为 v1.0 兼容 API 保留。

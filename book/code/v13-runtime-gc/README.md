# v0.13 · Runtime 所有权与完整 GC

本快照把 v0.12 的 heap 对象从 `Rc<RefCell<_>>` 迁移到 `Vm.heap: Heap` 唯一拥有的 arena。Table、closure 和 upvalue 通过不可拥有对象的 `GcRef<T>` 访问；handle 同时保存 heap ID 与单调递增的 object ID。这里的 Runtime 指运行时所有权设计，本快照没有单独的 `Runtime` 类型；`Proto` 仍由 `Rc<Proto>` 管理。

```powershell
cargo run --manifest-path book/code/v13-runtime-gc/Cargo.toml -- `
  book/code/v13-runtime-gc/examples/demo.lua
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --workspace
```

`gc_collect()` 与 `gc_live()` 是本阶段的教学诊断入口。完整 mark/sweep 从栈帧、global、registry、活跃函数/Proto、open upvalue、primitive metatable 和临时调用值出发；不可达自环可以回收。宿主保留的 Value 应在显式回收前写入 registry。

新增的 [GC 可执行练习](exercises/README.md) 包含双表环 Lua starter、宿主 registry
保留期实现、CLOSE / primitive root 调试，以及部分 mark 失败后的恢复验收：

```powershell
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_reference
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_exercises -- --ignored
```

参考验收应有 6 项通过。学习者验收初始有 4 项 TODO 失败，完成后应有 4 项通过；
默认 workspace 测试跳过这 4 项，保持快照可构建。练习说明列出修改范围、单项命令、
预期失败和可选参考解；它们不改变本快照的 GC 语义。

本版本暂不支持增量 GC、write barrier、自动 checkpoint、weak table、finalizer、userdata 和 coroutine。标准 `collectgarbage` 属于后续标准库版本；当前两个 `gc_*` 名字不会作为 v1.0 兼容 API 保留。

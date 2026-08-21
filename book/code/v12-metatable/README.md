# v0.12 · Metatable 与元方法

本快照在 v0.11 closure/upvalue 的基础上加入动态语义分发。Table 可以持有 metatable；索引、写入、算术、比较、拼接、长度、调用和字符串转换在原始快速路径失败后查找对应元方法。

```powershell
cargo run --manifest-path book/code/v12-metatable/Cargo.toml -- `
  --dump-bytecode book/code/v12-metatable/examples/demo.lua
cargo test --manifest-path book/code/v12-metatable/Cargo.toml --workspace
```

本版提供 `setmetatable`、`getmetatable`、`rawget`、`rawset`、`tostring`，并用教学接口 `settypemetatable` 演示 primitive 共用 metatable。`__index`、`__newindex` 与 `__call` 链最多递归 64 层，超限会返回 `VmErrorKind::Limit`。

兼容性提示：为了在本阶段完整展示长度元方法，table 的 `__len` 是教学扩展；官方 Lua 5.1.5 对 table 仍使用原始长度。`settypemetatable` 也不是 Lua 5.1 base library API，后续会迁移到 debug/宿主边界。这两点都不能在 v1.0 静默保留。

本版本暂不支持 GC、weak table、finalizer、userdata、coroutine 和完整标准库。Table、closure、upvalue 与 metatable 仍由 `Rc` 强引用，环无法回收；v0.13 将改为 Runtime 管理的 `GcRef`。

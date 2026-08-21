# v0.10 · 多返回、vararg 与迭代器

上一版只有“每次调用恰好得到一个值”的协议。本快照把调用窗口升级为 Lua 风格的固定/开放结果协议，并在同一条数据通路上实现：

- 多返回值在赋值、参数、`return` 和 table 构造器末尾的展开/折叠规则；
- `...` 参数的截取、转发与返回；
- `TAILCALL` 和复用 Lua 调用帧的 trampoline；
- generic for，以及教学阶段内建的 `next`、`pairs`、`ipairs` 迭代三元组。

```powershell
cargo run --manifest-path book/code/v10-multi-return-vararg/Cargo.toml -- `
  --dump-bytecode book/code/v10-multi-return-vararg/examples/demo.lua
cargo test --manifest-path book/code/v10-multi-return-vararg/Cargo.toml --workspace
```

本版本暂不支持：捕获外层 local、metatable/元方法、完整标准库与 GC。`next`、`pairs`、`ipairs` 目前是 VM 内建函数；v0.16 会把它们迁入统一的 Rust 宿主函数注册边界。

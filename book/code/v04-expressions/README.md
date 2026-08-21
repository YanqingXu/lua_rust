# v0.4 · 完整表达式语义

这个快照在 AST 求值器中实现 Lua 算术、比较、逻辑短路、拼接和长度语义。`and/or` 返回原值，且只有 `nil` 和 `false` 为假。

```powershell
cargo run --manifest-path book/code/v04-expressions/Cargo.toml -- book/code/v04-expressions/examples/demo.lua
cargo test --manifest-path book/code/v04-expressions/Cargo.toml
```

本版本暂不支持：语句、变量、Table、函数和字节码。v0.5 将删除 AST 直接执行路径，改用寄存器 VM。

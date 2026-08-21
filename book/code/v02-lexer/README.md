# v0.2 · Lexer 与 Token

这个快照将 Lua 源码字节流转换为带位置的 Token。它支持 Lua 5.1 关键字、名字、数字、长短字符串、注释和运算符。

```powershell
cargo run --manifest-path book/code/v02-lexer/Cargo.toml -- book/code/v02-lexer/examples/demo.lua
cargo test --manifest-path book/code/v02-lexer/Cargo.toml
```

本版本暂不支持：语法树、表达式求值、语句和字节码。

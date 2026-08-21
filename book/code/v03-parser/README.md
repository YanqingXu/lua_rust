# v0.3 · Parser 与最小 AST

这个快照使用 Pratt parser 把字面量、名字、括号、一元和二元表达式组装成带源码位置的 AST。求值器只实现了算术、`not` 和字符串长度，让 Parser 仍是本版本的唯一核心增量。

```powershell
cargo run --manifest-path book/code/v03-parser/Cargo.toml -- book/code/v03-parser/examples/demo.lua
cargo test --manifest-path book/code/v03-parser/Cargo.toml
```

本版本暂不支持：完整表达式语义、语句、作用域和字节码。

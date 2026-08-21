# v0.6 · 变量、赋值与作用域

这个快照在 v0.5 寄存器 VM 上新增 statement AST、global/local、普通与多重赋值、`do ... end` 作用域和局部遮蔽。

```powershell
cargo run --manifest-path book/code/v06-variables-scope/Cargo.toml -- `
  --dump-bytecode book/code/v06-variables-scope/examples/demo.lua
cargo test --manifest-path book/code/v06-variables-scope/Cargo.toml --workspace
```

本版本暂不支持：条件、循环、Table、函数和多返回展开。

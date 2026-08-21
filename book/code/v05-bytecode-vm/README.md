# v0.5 · Bytecode 与最小寄存器 VM

这个快照将 v0.4 的 AST 直接执行迁移为 `AST → Codegen → Proto → VM`，并第一次拆成 `lua_core`、`lua_compiler`、`lua_vm` 和 `lua_app` 四个 crate。

```powershell
cargo run --manifest-path book/code/v05-bytecode-vm/Cargo.toml -- `
  --dump-bytecode book/code/v05-bytecode-vm/examples/demo.lua
cargo test --manifest-path book/code/v05-bytecode-vm/Cargo.toml --workspace
```

本版本暂不支持：语句、变量、Table、函数和完整 Lua 5.1 opcode 集。

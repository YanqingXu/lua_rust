# v0.7 · 控制流

这个快照使用显式跳转和回填实现 `if/elseif/else`、`while`、`repeat until`、numeric `for`、`break` 与嵌套 block。

```powershell
cargo run --manifest-path book/code/v07-control-flow/Cargo.toml -- `
  --dump-bytecode book/code/v07-control-flow/examples/demo.lua
cargo test --manifest-path book/code/v07-control-flow/Cargo.toml --workspace
```

本版本暂不支持：generic for、Table、函数和调用。numeric for 对 step=0 返回结构化运行时错误。

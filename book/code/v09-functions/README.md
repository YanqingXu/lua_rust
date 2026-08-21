# v0.9 · 函数与调用帧

这个快照新增函数声明、local function、函数表达式、参数、单返回、递归、方法声明与冒号调用。

```powershell
cargo run --manifest-path book/code/v09-functions/Cargo.toml -- `
  --dump-bytecode book/code/v09-functions/examples/demo.lua
cargo test --manifest-path book/code/v09-functions/Cargo.toml --workspace
```

本版本暂不支持：多返回、vararg、tail call 与 closure capture。嵌套函数可用，但访问外层 local 将到 v0.11 通过 Upvalue 实现。

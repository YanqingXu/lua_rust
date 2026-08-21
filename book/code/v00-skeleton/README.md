# v0.0：最小解释器骨架

这一版只建立输入、执行器边界与错误出口，不解析 Lua。

```powershell
cargo run --manifest-path book/code/v00-skeleton/Cargo.toml -- `
  book/code/v00-skeleton/examples/demo.lua
cargo test --manifest-path book/code/v00-skeleton/Cargo.toml
```

预期输出包含脚本路径与 `75` 个原始字节。下一版会让执行器第一次理解 Lua 字面量。

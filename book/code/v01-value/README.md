# v0.1：Lua Value 与字面量

这一版第一次理解 Lua 值，支持 nil、boolean、number 与任意字节字符串。

```powershell
cargo run --manifest-path book/code/v01-value/Cargo.toml -- `
  book/code/v01-value/examples/demo.lua
cargo test --manifest-path book/code/v01-value/Cargo.toml
```

示例会证明 `\000` 与 `\255` 没有被 UTF-8 转换吞掉。本版仍只接受一个字面量；
v0.2 将用完整 Token 流替换临时读取器。

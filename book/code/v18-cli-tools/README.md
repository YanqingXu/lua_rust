# v0.18 · CLI、REPL 与字节码工具

本快照完成 Lua 风格脚本、stdin、`-e/-l/-i/--`、arg table 与持久 REPL，并新增 `lua_bytecode` crate。工具可从同一 Proto 生成 text、稳定 JSON、CFG DOT，或执行后输出逐指令 trace JSON。

```powershell
cargo run --manifest-path book/code/v18-cli-tools/Cargo.toml -- `
  book/code/v18-cli-tools/examples/demo.lua
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format cfg book/code/v18-cli-tools/examples/demo.lua
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace
```

CLI 的 usage error 返回 2，compile/runtime/IO error 返回 1，Lua `print` 与脚本副作用保持 stdout 合同。REPL 支持 `=expression` 和未闭合 block 的续行。

fixture manifest 与 process runner 对 stdout/stderr/exit code/timeout 做黑盒验收。v1.0 前仍需补齐语言/标准库差异并建立完整 oracle gate。

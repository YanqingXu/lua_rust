# v0.16 · 核心标准库

本快照新增 `lua_stdlib` crate，集中管理 Rust builtin 目录、Lua coercion 和一基参数错误。全局环境成为真正的 Heap table，`_G`、普通 global 和 math/string/table/coroutine 库共享同一个对象。

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml -- `
  book/code/v16-core-stdlib/examples/demo.lua
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --workspace
```

本版公开 19 个 base、28 个 math、10 个 byte-string、5 个 table 与 6 个 coroutine 入口。`pcall/xpcall`、string method、table comparator、`collectgarbage` 以及 `_VERSION`/`_G` 都有端到端合同测试，并在存在 `lua` 命令时运行代表性 Lua 5.1 差分。

Lua pattern、完整 `string.format`、chunk loader、function environment 和部分 collector tuning 仍留待后续兼容收口；io/os/package/debug 属于 v0.17。

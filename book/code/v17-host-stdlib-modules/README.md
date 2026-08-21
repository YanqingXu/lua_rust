# v0.17 · 宿主标准库与 Lua 模块

本快照把标准库边界扩展到宿主资源：文件 userdata 有独立 metatable，并在 close、GC 或 VM shutdown 时释放；os API 统一返回平台错误。VM 直接复用编译器实现 `loadstring/loadfile/dofile` 与 Lua 文件模块。

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- `
  book/code/v17-host-stdlib-modules/examples/demo.lua
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --workspace
```

`require` 支持 `package.path`、`package.preload`、`package.loaded`、缓存、循环 sentinel 与失败回滚；debug 库可查看 metatable、registry、Proto 信息和 upvalue。动态 native module 被明确排除。

io 默认流切换/lines、完整 os locale/date、package.loadlib、debug locals/hooks/function environment 仍留待兼容收口。

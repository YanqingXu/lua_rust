# v1.0 · Lua 5.1 语言运行时兼容收口

本快照把 v0.18 的“功能存在”升级为可审计的发布合同：补齐 Lua 调用语法糖，冻结 bytecode/trace schema，逐项映射官方 38 Opcode，并用闭世界能力矩阵、偏差注册表和双 oracle runner 阻止静默近似。

```powershell
cargo run --manifest-path book/code/v100-lua51-runtime/Cargo.toml -- `
  book/code/v100-lua51-runtime/examples/demo.lua
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace
powershell -NoProfile -ExecutionPolicy Bypass -File `
  book/code/v100-lua51-runtime/tools/release_gate.ps1
```

示例 stdout 为 `100`。

## 发布合同

- `compatibility/capabilities.tsv` 是能力真相源；只有 `complete`、`partial`、`excluded` 三种状态。
- `compatibility/deviations.tsv` 是闭世界偏差注册表；每个非完整能力必须引用唯一 ID，每个 ID 也必须被能力矩阵引用。
- `compatibility/oracles.toml` 固定官方 Lua 5.1.5 源码/测试归档哈希、C++ oracle commit 与归档哈希。
- `compatibility/differential-baseline.json` 记录 2026-08-21 实际完成的 5/5 × 双 oracle 基线。
- `lua_bytecode --format opcode-audit` 输出 38 行机器可读映射，并明确 `binary_chunk_compatible=false`。
- bytecode JSON 与 VM trace 分别冻结为 `lua-rust-bytecode/v1` 和 `lua-rust-vm-trace/v1`。

真实双 oracle gate：

```powershell
book/code/v100-lua51-runtime/tools/run_differential.ps1 `
  -OfficialLua path/to/lua51.exe `
  -CppLua path/to/lua_app.exe `
  -CppRoot path/to/lua_cpp-checkout
```

若 Git 传输不可用，可使用 `oracles.toml` 固定 SHA-256 的 exact-commit codeload archive，并额外传入 `-CppArchive`。

## 兼容性口径

完整支持的主线包括 Lua 5.1 byte-oriented 源码、表达式与语句、Table、函数、多返回、vararg、闭包、元方法、完整/增量 GC、弱表、Finalizer、Coroutine、Lua 文件模块核心链路、CLI、REPL 和观察工具。

标准库中的剩余差异不是隐藏实现：pattern/string.format、函数环境、若干 io/os/package/debug API、平台时间/locale、C `rand` 序列等均登记在 `LUA51-D004`–`LUA51-D011`，能力矩阵标为 `partial`。

## v1.0 明确不包含

- Lua 5.1 binary chunk 与官方指令二进制编码：v1.1；
- 稳定 C API/ABI、headers、lauxlib 和 C consumer：v1.2；
- `package.loadlib` 与动态原生模块：v1.3；
- 沙箱、配额、跨平台 SDK 和生产性能治理：v1.4。

`cargo test` 不依赖外部 oracle，保证快照可独立运行；`release_gate.ps1 -RunOracles` 是具有外部 provenance 的增强发布门。

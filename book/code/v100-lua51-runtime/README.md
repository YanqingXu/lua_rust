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

## 结构化 REPL 的独立迁移

本快照显式迁入 v0.18 的 `CompileErrorKind`、`Repl<'a>` 与真实 LF/EOF driver，
所有 Rust 源码与 path 依赖仍在本目录。保留本版括号 prefix、string/table call
sugar、`Action::Help` 顺序、arg 表、v1.0 banner 与 versioned JSON/trace。

- NeedMoreInput 保留 source 与原诊断，不执行前缀；编译成功后执行恰一次。
- 普通编译/runtime 错误清当前输入后继续，同一 Session 与既有副作用保留。
- idle EOF 为0；pending EOF 清 source、take 原诊断一次，由顶层诊断并退出1。
- 实际带 LF 才恢复换行；无 LF 末片段不合成 LF。读取 Err 是 Host 错误。
- kind 是局部构造提示，每次重编译整个 buffer；非增量 parser 或任意前缀可补全证明。

从仓库根运行[迁移包](exercises/repl_migration/README.md)与本地合同：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example repl_migration_problem_chain -- all
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_byte_contract real_binary_matches_all_shared_raw_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_exercises migrated_eof_consumes_original_diagnostic_once -- --ignored --exact
py -3 tools/check_repl_byte_contracts.py --check
```

固定示例12条 `[ok]`、6项参考、6项真实 controller/I/O 测试；learner 一个 EOF
TODO 初始 ignored，显式命令应失败，补完后通过。默认 workspace 为 **158通过、
1 ignored**，ignored 不表示已完成练习。Release 对同一 run/test 加 `--release`。

本地 byte 合同26个 case，与 v0.18 各自对固定 oracle 验完整 stdout/stderr bytes
与退出码；两份 fixture、wrapper、runner 逐 byte 相同，只有显式版本 banner 参数。
本地测试可随本目录独立复制构建；仓库身份检查与独立进程归档入口见
[共享合同说明](../../../tools/repl_byte_contracts/README.md)。不 trim/normalize 输出，
不把相互一致当作正确。read_line 仍使用 UTF-8；Lua print 走进程 stdout，泛型
writer 只接收 REPL 输出。TTY 编辑、signal、完整官方 REPL 等价不在26条合同内。

## 发布合同

- `compatibility/capabilities.tsv` 是能力真相源；只有 `complete`、`partial`、`excluded` 三种状态。
- `compatibility/deviations.tsv` 是闭世界偏差注册表；每个非完整能力必须引用唯一 ID，每个 ID 也必须被能力矩阵引用。
- `compatibility/oracles.toml` 固定官方 Lua 5.1.5 源码/测试归档哈希、C++ oracle commit 与归档哈希。
- `compatibility/differential-baseline.json` 记录 2026-08-21 实际完成的 5/5 × 双 oracle 基线。
- `lua_bytecode --format opcode-audit` 输出 38 行机器可读映射，并明确 `binary_chunk_compatible=false`。
- bytecode JSON 与 VM trace 分别冻结为 `lua-rust-bytecode/v1` 和 `lua-rust-vm-trace/v1`。
- 本地 REPL 续行与明确 EOF 策略由 `tests/repl_byte_contract.rs` 指向固定字节证据。

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

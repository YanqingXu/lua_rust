# lua_rust - Lua 5.1 Rust Interpreter

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是 `lua_cpp` 的 Lua 5.1 解释器 Rust 迁移工程。当前已经不只是脚手架：编译器、寄存器 VM、运行时核心、标准库目录和命令行入口已经打通，可以运行包含函数、闭包、表、循环、多返回、vararg、metatable、coroutine 和大量标准库调用的 Lua 脚本。

本项目仍不是完整 Lua 5.1 发行版替代品。C API、官方二进制 chunk、动态 C 模块加载、完整一致性测试和 C++ 基线自动对齐仍未完成。

最近代码审计日期：2026-07-05。

---

## 当前进度

| 范围 | 状态 | 真实进度 |
|---|---|---|
| Phase 0: Project Infrastructure | ✅ 已完成 | 6 crate workspace、CI workflow、PowerShell 质量脚本、docs、`tests/lua` 与 `tests/unit` 目录均已建立。 |
| Phase 1: Runtime Core | ✅ 核心完成 | `Value`、GC header/ref、mark-sweep GC、string pool、table、function/proto/upvalue、userdata、thread、metatable 均有实现和测试。 |
| Phase 2: Compiler | ✅ 大体可用 | Lexer、Parser、AST、38 opcode 编码、CodeGen 管线可生成可执行 `Proto`，覆盖函数/闭包/表构造/循环/多返回/vararg/方法调用等。 |
| Phase 3: VM | ✅ 大体可用 | 38 opcode dispatch、Lua/C 调用、tail call、闭包/upvalue、vararg、多返回、generic for、metatable 主要路径、coroutine resume/yield 路径已跑通。 |
| Phase 4: Standard Library | 🟡 大量实现，非完整兼容 | base/math/string/table/io/os/coroutine/debug/package 均注册并有集成测试；若干行为是项目内近似实现。 |
| Phase 5: CLI / tools | 🟡 可用但朴素 | `lua_app` 支持脚本、stdin、`-e`、`-l`、`-i`、REPL、`arg`；`lua_bytecode` 支持 text/JSON dump。 |

---

## 已验证能力

### 语言与 VM

- 基础表达式：nil/boolean/number/string、算术、比较、逻辑、取长、字符串拼接。
- 变量与作用域：global/local/block local、赋值、多重赋值、索引赋值、字段赋值。
- 控制流：`if/elseif/else`、`while`、`repeat until`、`break`、`do`、numeric for、generic for。
- 函数系统：函数声明、局部函数、函数表达式、方法定义/调用、递归、tail call。
- 闭包与 upvalue：嵌套闭包、共享 upvalue、关闭 open upvalue。
- 调用语义：Lua 函数、C 函数、多返回、最终调用展开、非最终多返回折叠、vararg、Lua 5.1 风格 `arg` 表。
- 表：数组/哈希/混合构造器、大数组 `SETLIST`、成员访问、索引访问、`next/pairs/ipairs`。
- Metatable：`__index`、`__newindex`、`__call`、算术、拼接、比较、`__len`、`__tostring`、弱表和 `__gc` 的关键路径。
- Coroutine：`create/resume/yield/status/running/wrap` 的基本流程可用。

### 标准库

| 模块 | 已注册能力 |
|---|---|
| base | `assert`、`collectgarbage`、`dofile`、`error`、`gcinfo`、`getfenv`、`getmetatable`、`ipairs`、`load`、`loadfile`、`loadstring`、`newproxy`、`next`、`pairs`、`pcall`、`print`、`rawequal`、`rawget`、`rawset`、`select`、`setfenv`、`setmetatable`、`tonumber`、`tostring`、`type`、`unpack`、`xpcall` |
| math | `abs`、`acos`、`asin`、`atan`、`atan2`、`ceil`、`cos`、`cosh`、`deg`、`exp`、`floor`、`fmod/mod`、`frexp`、`ldexp`、`log`、`log10`、`max`、`min`、`modf`、`pow`、`rad`、`random`、`randomseed`、`sin`、`sinh`、`sqrt`、`tan`、`tanh`、`huge`、`pi` |
| string | `byte`、`char`、`dump`、`find`、`format`、`gmatch/gfind`、`gsub`、`len`、`lower`、`match`、`rep`、`reverse`、`sub`、`upper`；包含一套 Lua pattern 近似实现。 |
| table | `concat`、`foreach`、`foreachi`、`getn`、`insert`、`maxn`、`remove`、`sort` |
| io | `tmpfile`、`open`、`input`、`output`、`read`、`write`、`lines`、`flush`、`close`、`type`，以及文件句柄的 `read/write/seek/close/setvbuf/lines/flush`。 |
| os | `clock`、`date`、`difftime`、`execute`、`remove`、`rename`、`setlocale`、`time`、`tmpname` |
| coroutine | `create`、`resume`、`running`、`status`、`wrap`、`yield` |
| debug | `getinfo`、`getupvalue`、`setupvalue`、`getlocal`、`setlocal`、`gethook`、`sethook`、`traceback`、`getregistry`、`getfenv`、`setfenv`、`setmetatable` |
| package | `require`、`module`、`package.loaded`、`package.preload`、`package.path`、`package.loadlib` 占位错误返回、`package.seeall` |

---

## 测试状态

2026-07-05 本地执行：

```powershell
cargo test --workspace
```

结果：✅ 全部通过，593 个 Rust 测试通过，0 失败。doc-tests 当前为 0。

| Crate | 单元测试 | 集成测试 | 总计 |
|---|---:|---:|---:|
| `lua_core` | 258 | 41 | 299 |
| `lua_compiler` | 115 | 66 | 181 |
| `lua_vm` | 6 | 29 | 35 |
| `lua_stdlib` | 0 | 78 | 78 |
| `lua_app` | 0 | 0 | 0 |
| `lua_bytecode` | 0 | 0 | 0 |
| **总计** | **379** | **214** | **593** |

额外命令行 smoke test：

```powershell
cargo run -q -p lua_app -- examples\more_tests.lua
cargo run -q -p lua_app -- -e "local t={3,1,2}; table.sort(t); print(table.concat(t, ','))"
cargo run -q -p lua_app -- -e "print(string.gsub('a1 b2','(%a)(%d)','%2%1'))"
cargo run -q -p lua_bytecode -- examples\more_tests.lua
```

均可正常运行；`lua_bytecode` 对 `examples/more_tests.lua` 输出 27 条指令、14 个常量。

---

## 已知限制

| 领域 | 当前限制 |
|---|---|
| Lua 5.1 完整兼容性 | 尚未接入官方 Lua 5.1 测试套件；当前通过的是项目内单元/集成测试。 |
| C++ 基线对齐 | `.github/workflows/ci.yml` 中 `cross-validate` 仍为 `if: false`；bytecode/VM trace 与 `lua_cpp` 的自动对齐未启用。 |
| C API / FFI | 没有 Lua C API crate；不能作为完整嵌入式 Lua ABI 使用。`package.loadlib` 明确返回“不支持动态库”。 |
| 二进制 chunk | `string.dump` 使用项目内 in-process dump registry，不是官方 Lua 5.1 二进制 chunk 格式；`lua_bytecode` 只是调试 dump。 |
| GC 与运行时内部 | mark-sweep 路径可用，但增量 GC strategy 仍是占位；多处 write barrier 注释仍待真正接入；coroutine 的 `LuaState` 通过裸指针挂在 `Thread` 上。 |
| 标准库边界 | `io/os/debug/package/string pattern` 是以测试覆盖为目标的近似实现，不保证覆盖所有 Lua 5.1 边角行为和平台差异。 |
| 编译器错误恢复 | Parser/CodeGen 已能处理大量语法，但错误恢复仍有限，极端长跳转仍可能触发内部 panic。 |
| CLI 体验 | REPL 可用并支持不完整输入续行，但没有 readline/history 等成熟交互体验。 |
| 工具显示 | `lua_bytecode` 当前不会展示字符串常量内容，只显示 `string` 类型。 |

---

## 快速开始

```powershell
cd lua_rust

cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace

# 运行 Lua 脚本
cargo run -p lua_app -- examples\more_tests.lua

# 执行一段源码
cargo run -p lua_app -- -e "print(math.sqrt(81))"

# 进入 REPL
cargo run -p lua_app

# 字节码 dump
cargo run -p lua_bytecode -- examples\more_tests.lua
cargo run -p lua_bytecode -- examples\more_tests.lua --format=json
```

---

## Crate 说明

| Crate | 类型 | 职责 | 当前测试 |
|---|---|---|---:|
| `lua_core` | lib | 类型系统、GC、字符串池、Table/Metatable/Proto/Function/Upvalue/Userdata/Thread。 | 299 |
| `lua_compiler` | lib | Opcode、Lexer/Token、AST、Parser、CodeGen。 | 181 |
| `lua_vm` | lib | LuaState、Stack、CallInfo、opcode dispatch、调用/返回/协程/元方法执行。 | 35 |
| `lua_stdlib` | lib | base/math/string/table/io/os/coroutine/debug/package 标准库实现。 | 78 |
| `lua_app` | bin | Lua 5.1 命令行 runner、stdin、`-e/-l/-i`、脚本参数、REPL。 | 0 |
| `lua_bytecode` | bin | Lua 源码到 `Proto` 的 text/JSON 字节码查看器。 | 0 |

---

## CI 与质量门

`.github/workflows/ci.yml` 当前配置：

| Job | 状态 | 内容 |
|---|---|---|
| `quality-gate` | 已配置 | Windows 上运行环境检查、fmt、clippy、build、nextest、doc、audit、质量脚本。 |
| `cross-validate` | 未启用 | `if: false`；预留给 C++ bytecode/VM trace 对齐。 |

本地这次审计只执行并确认了 `cargo test --workspace` 与若干 CLI smoke test；未重新跑完整 clippy/doc/audit 质量门。

---

## 文档

| 文档 | 说明 |
|---|---|
| [Phase 0 报告](docs/rust_migration/phase_0_report.md) | 基础设施初始化的历史报告。 |
| [偏差日志](docs/rust_migration/deviation_log.md) | Rust 与 C++ 基准之间已批准行为偏差的登记表；目前仍为空。 |
| [类型映射表](docs/rust_migration/type_mapping_table.md) | C++ → Rust 类型映射速查表；部分内容仍是迁移初期快照。 |
| [P1.1 任务说明](docs/rust_migration/tasks/P1.1-types-value.md) | Phase 1.1 类型/Value 任务历史文档。 |
| [术语表](docs/glossary.md) | Lua 概念与项目术语说明。 |

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。

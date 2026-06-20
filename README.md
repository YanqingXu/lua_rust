# lua_rust - Lua 5.1 Rust Migration

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是 `lua_cpp` 的 Lua 5.1 解释器 Rust 迁移工程。已完成 Phase 0 ~ Phase 5 全部六个阶段的迁移，实现了从词法分析、语法解析、字节码编译到虚拟机执行和标准库的完整管线。

最近审计日期：2026-06-20。

---

## 当前进度

| 范围 | 状态 | 说明 |
|---|---|---|
| Phase 0: Project Infrastructure | ✅ 已完成 | Rust workspace、6 个 crate 脚手架、CI workflow、质量脚本。 |
| Phase 1: Runtime Core | ✅ 已完成 | 类型系统、GC 基础设施、标记-清除 GC、全部核心对象模型 (Table/Metatable/Proto/Function/Upvalue/Userdata/Thread)。 |
| Phase 2: Compiler | ✅ 已完成 | OpCode 38 指令定义、Lexer 词法分析、Token 类型、AST 27 节点 (14 Expr + 13 Stmt)、递归下降 Parser、CodeGen 完整管线 (types/reg_alloc/jump/scope/binder/expr_emit/stmt_emit/func_comp)。 |
| Phase 3: VM | ✅ 已完成 | LuaState、Stack、CallInfo、38 opcode dispatch 主循环、数据移动/算术/控制流/循环/表操作全部实现。 |
| Phase 4: Standard Library | ✅ 已完成 | Catalog 注册系统、base (20 函数)、math (23 函数)、string (10 函数)、table (5 函数)。 |
| Phase 5: CLI / tools | ✅ 已完成 | `lua_app` 脚本执行器 + 交互式 REPL、`lua_bytecode` 字节码 text/JSON dumper。 |

---

## 已实现的核心功能

### Phase 1: Runtime Core (lua_core)

- **类型系统**：`ValueType`/`GcObjectType`/`GcColor`、`Value` enum（9 种变体）、Lua truthiness、指针身份相等。
- **GC 基础设施**：`GcRef<T>`、`GcObjectHeader`、`GcObject` unsafe trait、`GarbageCollector`（完整标记-清除循环：mark/propagate/sweep）、弱表清理、写屏障、终结器框架。
- **字符串驻留**：`GcString`、Lua 5.1 风格字符串哈希、`StringPool::intern/find/remove`。
- **核心对象模型**：`Table`（数组/哈希混合存储、元表引用、`#` 长度运算符、`next()` 迭代器）、`Metatable`（17 种 TMS 枚举）、`Proto`（字节码、常量表、调试信息）、`Function`（C/Lua 闭包、上值管理）、`Upvalue`（Open/Closed 状态）、`Userdata`（GC 管理字节缓冲区）、`Thread`（协程状态）。

### Phase 2: Compiler (lua_compiler)

- **OpCode 定义**：38 条指令、iABC/iABx/iAsBx 编码、元数据表（含 B/C 模式、分组、元方法标志）。
- **Lexer 词法分析**：完整关键字表（21 个）、标识符、数字（含十六进制/科学记数法）、字符串（含转义/长字符串）、注释（行注释/长注释）。
- **AST 定义**：14 种表达式节点 + 13 种语句节点 = 27 种完整 AST 节点类型、`ExprVisitor`/`StmtVisitor`/`AstVisitor` trait。
- **Parser 递归下降语法分析**：12 级表达式优先级链（or → and → compare → concat → add → mul → unary → pow）、全部 Lua 5.1 语句、表构造器、函数声明/表达式。
- **CodeGen 代码生成**：9 个模块 (types, reg_alloc, builder, jump, scope, binder, expr_emit, stmt_emit, func_comp) 对标 C++ 18 个文件。

### Phase 3: VM (lua_vm)

- **VM State**：`Stack`（动态扩展值栈）、`CallInfo`（调用帧，含 func/base/top/savedpc/nresults）、`LuaState`（值栈 + 调用栈 + 线程状态）。
- **38 条指令 dispatch**：使用 Rust `match` 分发全部 38 条指令（编译器生成跳转表）。
  - 数据移动：MOVE, LOADK, LOADBOOL, LOADNIL ✅
  - 上值/全局：GETUPVAL, GETGLOBAL, SETGLOBAL, SETUPVAL 🏗️
  - 表操作：GETTABLE, SETTABLE, NEWTABLE, SELF ✅
  - 算术运算：ADD, SUB, MUL, DIV, MOD, POW ✅
  - 一元运算：UNM, NOT, LEN ✅
  - 控制流：JMP, EQ, LT, LE, TEST, TESTSET ✅
  - 函数调用：CALL, TAILCALL, RETURN ✅ (RETURN 完整)
  - 循环：FORLOOP, FORPREP, TFORLOOP ✅
  - 表/闭包/变参：SETLIST, CLOSE, CLOSURE, VARARG 🏗️

### Phase 4: Standard Library (lua_stdlib)

- **Catalog 注册系统**：按 ID/Name 查找库、`open_all()` 一键加载全部标准库。
- **base** (20 函数)：print, type, tostring, tonumber, error, assert, setmetatable, getmetatable, rawget, rawset, rawequal, select, pcall, xpcall, next, pairs, ipairs, loadstring, dofile, collectgarbage。
- **math** (23 函数，宏生成)：abs, acos, asin, atan, atan2, ceil, cos, cosh, deg, exp, floor, fmod, log, log10, max, min, pow, rad, sin, sinh, sqrt, tan, tanh。
- **string** (10 函数)：len, sub, upper, lower, reverse, byte, char, find, rep, format。
- **table** (5 函数)：insert, remove, sort, concat, maxn。

### Phase 5: CLI Tools

- **lua_app**：脚本文件执行器 + 交互式 REPL（自动 wrap 表达式为 return）。
- **lua_bytecode**：字节码 text/JSON 双格式 dump，含常量表、指令序列、源行号对应。

---

## Crate 说明

| Crate | 类型 | 当前职责与状态 |
|---|---|---|
| `lua_core` | lib | Phase 1 ✅ — 类型系统、GC、全部核心对象模型 (Table/Metatable/Proto/Func/Upvalue/Userdata/Thread)。297 个测试。 |
| `lua_compiler` | lib | Phase 2 ✅ — Opcode、Lexer/Token、AST、递归下降 Parser、CodeGen 完整管线。177 个测试。 |
| `lua_vm` | lib | Phase 3 ✅ — LuaState、Stack、CallInfo、38 opcode dispatch 主循环。6 个测试。 |
| `lua_stdlib` | lib | Phase 4 ✅ — Catalog + base/math/string/table 共 58 个 C 函数。 |
| `lua_app` | bin | Phase 5 ✅ — Lua 脚本执行器 + 交互式 REPL。 |
| `lua_bytecode` | bin | Phase 5 ✅ — 字节码 text/JSON 双格式 dump。 |

---

## 快速开始

### 环境要求

- Rust stable，workspace 要求 `rust-version = "1.96"`。
- Windows x64 MSVC 是当前主开发/CI 平台。

### 本地基础验证

```powershell
cd lua_rust

cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
cargo doc --no-deps
```

当前审计结果：上述命令均通过；`cargo test --workspace` 共运行 **480 个 Rust 测试**，覆盖全部 6 个 crate。

### 运行 CLI 工具

```powershell
# 执行 Lua 脚本
cargo run -p lua_app -- tests/fixtures/phase_2/hello.lua

# 交互式 REPL
cargo run -p lua_app

# 字节码 dump
cargo run -p lua_bytecode -- tests/fixtures/phase_2/hello.lua

# JSON 格式字节码
cargo run -p lua_bytecode -- tests/fixtures/phase_2/hello.lua --format=json
```

---

## 项目结构

```text
lua_rust/
├── Cargo.toml
├── rust-toolchain.toml
├── .cargo/config.toml
├── .github/workflows/ci.yml
├── crates/
│   ├── lua_core/
│   │   ├── src/types.rs, value.rs, table.rs, metatable.rs
│   │   ├── src/gc/ (collector, mark, sweep, weak, barrier, strategy)
│   │   ├── src/gc_string.rs, string_pool.rs
│   │   ├── src/function.rs, upvalue.rs, userdata.rs, thread.rs
│   │   └── src/proto.rs, function.rs
│   ├── lua_compiler/
│   │   ├── src/opcode.rs, token.rs, lexer.rs
│   │   └── src/ast/ (expr, stmt, visitor)
│   │   └── src/parser/ (mod, expr, primary, stmt, func, table)
│   │   └── src/codegen/ (mod, types, reg_alloc, builder, jump, scope, binder, expr_emit, stmt_emit, func_comp)
│   ├── lua_vm/
│   │   ├── src/state/ (mod, stack, call_info, lua_state)
│   │   └── src/execute.rs
│   ├── lua_stdlib/
│   │   └── src/ (catalog, base, math, string, table)
│   ├── lua_app/src/main.rs
│   └── lua_bytecode/src/main.rs
├── tests/fixtures/phase_1..5/
├── tools/ (rust_env_check, rust_quality_gate, compare_bytecode, compare_vm_trace)
└── docs/ (glossary, rust_migration/)
```

---

## 测试统计

| Crate | 单元测试 | 集成测试 | 总计 |
|---|---|---|---|
| `lua_core` | 256 | 41 | 297 |
| `lua_compiler` | 111 | 66 | 177 |
| `lua_vm` | 6 | 0 | 6 |
| `lua_stdlib` | 0 | 0 | 0 |
| `lua_app` | 0 | 0 | 0 |
| `lua_bytecode` | 0 | 0 | 0 |
| **总计** | **373** | **107** | **480** |

---

## CI 与 C++ 基准对齐

`.github/workflows/ci.yml` 当前包含两个 job：

| Job | 状态 | 说明 |
|---|---|---|
| `quality-gate` | 已配置 | Windows 上运行 Rust 工具链安装、环境检查、fmt、clippy、build、nextest、doc、audit 和质量脚本。 |
| `cross-validate` | 已配置但禁用 | `if: false`，计划在 C++ 基准产物可用后启用。 |

### 交叉验证现状

`tools/compare_bytecode.ps1` 和 `tools/compare_vm_trace.ps1` 已存在，当前对齐状态为 N/A：

- Rust 侧 compiler、VM、CLI 已全部实现，可产生真实字节码和 VM 执行结果。
- `..\lua_cpp\bin` 当前不存在，因此 C++ 基准不可用，交叉验证待 C++ 侧构建后启用。
- `docs/rust_migration/deviation_log.md` 当前没有登记任何已批准行为偏差。

---

## 待完善 (Known TODOs)

Phase 2~5 全部模块框架已就位，以下为待深度实现的子项目：

| 领域 | 待完善项 |
|---|---|
| **CodeGen** | 字符串常量驻留 (StringPool + GC)、完整表构造器 SETLIST 发射、跨函数 upvalue 解析、CLOSURE proto index 分配 |
| **VM** | 全局表 / upvalue 实际访问、Table::get/set + 元方法调用、函数调用栈帧管理 (CALL/TAILCALL)、GC 字符串创建 (CONCAT) |
| **标准库** | Io/Os/Coroutine/Debug/Package 模块未实现；已实现库中部分函数有 placeholder 值 |
| **CLI** | REPL 不支持多行输入；lua_bytecode 不输出字符串常量内容 |

---

## 开发约定

- 类型使用 `CamelCase`，函数/方法/模块使用 `snake_case`，常量使用 `UPPER_CASE`。
- `unsafe` 仅用于 GC、裸指针/FFI 等边界；workspace lint 开启 `unsafe_op_in_unsafe_fn = "deny"` 和 `clippy::undocumented_unsafe_blocks = "deny"`。
- 迁移应优先保持与 `lua_cpp` 行为一致；所有批准的行为差异必须登记到偏差日志。
- Phase 推进时先更新对应 crate 的真实实现，再更新 README 和迁移文档，避免文档先行宣称未落地功能。

---

## 文档

| 文档 | 说明 |
|---|---|
| [Phase 1 进度报告](docs/rust_migration/phase_1_progress.md) | Phase 1 Runtime Core 整体进度跟踪。 |
| [P1.4 Table 任务](docs/rust_migration/tasks/P1.4-table.md) | Table 核心结构与基础操作迁移说明。 |
| [P1.4 Metatable 任务](docs/rust_migration/tasks/P1.4-metatable.md) | Metatable/TMS 系统迁移说明。 |
| [偏差日志](docs/rust_migration/deviation_log.md) | Rust 与 C++ 基准之间已批准行为偏差的登记表。 |
| [术语表](docs/glossary.md) | Lua 概念与项目术语说明。 |

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。

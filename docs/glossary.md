---
status: current
last_checked: 2026-07-07
applies_to: Lua 5.1.5 terminology used by this Rust interpreter
---

# Glossary

这份术语表说明项目中常见的 Lua 概念、Rust 类型和主要入口文件。它不是完整规范，而是读源码时的地图。

## Core Terms

| 术语 | Rust 类型/模块 | 入口文件 | 说明 |
|---|---|---|---|
| Value | `lua_core::value::Value` | `crates/lua_core/src/value.rs` | Lua 值的统一表示，覆盖 nil、boolean、number、string、table、function 等。 |
| StringPool | `lua_core::string_pool::StringPool` | `crates/lua_core/src/string_pool.rs` | 字符串驻留池。 |
| GarbageCollector | `lua_core::gc::collector::GarbageCollector` | `crates/lua_core/src/gc/collector.rs` | 标记-清除 GC 核心。 |
| GcStrategy | `lua_core::gc::strategy::GcStrategy` | `crates/lua_core/src/gc/strategy.rs` | GC 策略边界。 |
| Table | `lua_core::table::Table` | `crates/lua_core/src/table.rs` | Lua 表对象，承载数组部分、哈希部分和元表。 |
| Function | `lua_core::function::Function` | `crates/lua_core/src/function.rs` | 可执行函数对象，包含 Proto 或 C 函数。 |
| Proto | `lua_core::proto::Proto` | `crates/lua_core/src/proto.rs` | Lua 函数原型，保存字节码、常量表、子函数和调试信息。 |
| Upvalue | `lua_core::upvalue::Upvalue` | `crates/lua_core/src/upvalue.rs` | 闭包捕获的外层局部变量。 |
| Thread | `lua_core::thread::Thread` | `crates/lua_core/src/thread.rs` | 协程对象。 |
| Userdata | `lua_core::userdata::Userdata` | `crates/lua_core/src/userdata.rs` | 用户数据和可选终结器。 |
| Metatable | `lua_core::metatable` | `crates/lua_core/src/metatable.rs` | 元表与元方法管理。 |
| TMS | `lua_core::metatable::Tms` | `crates/lua_core/src/metatable.rs` | Tag Method System 元方法枚举。 |
| GcObject | `lua_core::gc::gc_object::GcObject` | `crates/lua_core/src/gc/gc_object.rs` | GC 管理对象的 unsafe trait。 |
| GCString | `lua_core::gc_string::GcString` | `crates/lua_core/src/gc_string.rs` | GC 管理的字符串对象。 |
| GcRef | `lua_core::gc::gc_ref::GcRef<T>` | `crates/lua_core/src/gc/gc_ref.rs` | 不透明 GC 引用句柄。 |
| OpCode | `lua_compiler::opcode::OpCode` | `crates/lua_compiler/src/opcode.rs` | Lua 5.1 的 38 条字节码指令枚举。 |
| Instruction | `lua_compiler::opcode::Instruction` | `crates/lua_compiler/src/opcode.rs` | 32-bit 指令编码。 |
| Lexer | `lua_compiler::lexer::Lexer` | `crates/lua_compiler/src/lexer.rs` | 词法分析器。 |
| Parser | `lua_compiler::parser::Parser` | `crates/lua_compiler/src/parser/mod.rs` | 语法分析器。 |
| AST | `lua_compiler::ast` | `crates/lua_compiler/src/ast/` | 抽象语法树。 |
| CodeGenerator | `lua_compiler::codegen::CodeGenerator` | `crates/lua_compiler/src/codegen/mod.rs` | 字节码生成器。 |
| LuaState | `lua_vm::state::LuaState` | `crates/lua_vm/src/state/lua_state.rs` | 单个 Lua 线程/协程的执行状态。 |
| Stack | `lua_vm::state::Stack` | `crates/lua_vm/src/state/stack.rs` | VM 执行时的值栈。 |
| CallInfo | `lua_vm::state::CallInfo` | `crates/lua_vm/src/state/call_info.rs` | 调用帧信息。 |
| VM | `lua_vm::execute` | `crates/lua_vm/src/execute.rs` | 字节码解释器。 |
| LibCatalog | `lua_stdlib::catalog` | `crates/lua_stdlib/src/catalog.rs` | 标准库表驱动注册入口。 |

## Naming Conventions

| 场景 | 约定 |
|---|---|
| 类型 | `CamelCase`，例如 `Value`、`GcRef`、`OpCode` |
| 函数和方法 | `snake_case`，例如 `is_false()`、`as_number()`、`execute_proto()` |
| 常量 | `UPPER_CASE`，例如 `LUA_MULTRET` |
| 模块 | `snake_case`，例如 `lua_core::gc`、`lua_compiler::codegen` |
| 可选值 | `Option<T>` |
| 错误传播 | `Result<T, E>` |
| 动态值集合 | Rust `enum`，例如 `Value`、`Expr`、`Stmt` |

## 推荐交叉阅读

- Rust 教学路线：`README.md`
- 类型速查：`docs/rust_migration/type_mapping_table.md`
- 行为说明日志：`docs/rust_migration/deviation_log.md`

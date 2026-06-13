---
status: current
verified_against: lua_cpp/docs/glossary.md; docs/roadmap/rust-migration-workflow.md
last_checked: 2026-06-13
applies_to: Lua terminology mapped across C++ and Rust implementations
---

# Glossary

这份术语表把 Lua 概念、C++ 仓库类型、Rust 迁移目标和主要文件放在一起。它不是完整规范，而是读代码时的地图。

## Core Terms

| 术语 | C++ 实现 | Rust 迁移目标 | 入口文件 (C++) | 入口文件 (Rust) | 说明 |
|---|---|---|---|---|---|
| Value | `Value` / `ValueVariant` | `lua_core::value::Value` | `src/core/value.hpp` | `crates/lua_core/src/value.rs` | Lua 值的统一表示，覆盖 nil、boolean、number、string、table、function 等。 |
| StringPool | `StringPool` | `lua_core::string_pool::StringPool` | `src/core/string_pool.hpp` | `crates/lua_core/src/string_pool.rs` | 字符串驻留池。 |
| GarbageCollector | `GarbageCollector` | `lua_core::gc::collector::GarbageCollector` | `src/gc/garbage_collector.hpp` | `crates/lua_core/src/gc/collector.rs` | 标记-清除 GC 核心。 |
| GCStrategy | `GCStrategy` / `MarkSweepGC` | `lua_core::gc::strategy::GcStrategy` | `src/gc/gc_strategy.hpp` | `crates/lua_core/src/gc/strategy.rs` | GC 策略边界。 |
| Table | `Table` | `lua_core::table::Table` | `src/core/table.hpp` | `crates/lua_core/src/table.rs` | Lua 表对象，承载数组部分、哈希部分和元表。 |
| Function | `Function` | `lua_core::function::Function` | `src/core/function.hpp` | `crates/lua_core/src/function.rs` | 可执行函数对象，包含 Proto 或 C 函数。 |
| Proto | `Proto` | `lua_core::function::Proto` | `src/core/function.hpp` | `crates/lua_core/src/function.rs` | Lua 函数原型，保存字节码、常量表、子函数、调试信息。 |
| Upvalue | `Upvalue` | `lua_core::upvalue::Upvalue` | `src/core/upvalue.hpp` | `crates/lua_core/src/upvalue.rs` | 闭包捕获的外层局部变量。 |
| Thread | `Thread` | `lua_core::thread::Thread` | `src/core/thread.hpp` | `crates/lua_core/src/thread.rs` | 协程包装。 |
| Userdata | `Userdata` | `lua_core::userdata::Userdata` | `src/core/userdata.hpp` | `crates/lua_core/src/userdata.rs` | 用户数据 + __gc 终结器。 |
| Metatable | `Table::getMetatable()` | `lua_core::metatable` | `src/core/metatable.hpp` | `crates/lua_core/src/metatable.rs` | 元表管理。 |
| TMS | `TMS` | `lua_core::metatable::Tms` | `src/core/metatable.hpp` | `crates/lua_core/src/metatable.rs` | Tag Method System 元方法枚举。 |
| GCObject | `GCObject` | `lua_core::gc::gc_object::GcObject` (trait) | `src/core/gc_object.hpp` | `crates/lua_core/src/gc/gc_object.rs` | GC 对象基类/unsafe trait。 |
| GCString | `GCString` | `lua_core::gc_string::GcString` | `src/core/gc_string.hpp` | `crates/lua_core/src/gc_string.rs` | GC 管理的字符串对象。 |
| GcRef | (raw pointer) | `lua_core::gc::gc_ref::GcRef<T>` | N/A | `crates/lua_core/src/gc/gc_ref.rs` | 不透明 GC 引用句柄（Rust 安全抽象）。 |
| OpCode | `OpCode` | `lua_compiler::opcode::OpCode` | `src/compiler/opcode.hpp` | `crates/lua_compiler/src/opcode.rs` | Lua 5.1 风格的 38 条字节码指令枚举。 |
| Instruction | `Instruction` | `lua_compiler::opcode::Instruction` | `src/compiler/opcode.hpp` | `crates/lua_compiler/src/opcode.rs` | 32-bit 指令编码。 |
| Lexer | `Lexer` | `lua_compiler::lexer::Lexer` | `src/compiler/lexer/lexer.hpp` | `crates/lua_compiler/src/lexer/mod.rs` | 词法分析器。 |
| Parser | `Parser` | `lua_compiler::parser::Parser` | `src/compiler/parser/parser.hpp` | `crates/lua_compiler/src/parser/mod.rs` | 语法分析器。 |
| AST | `Expr` / `Stmt` 派生节点 | `lua_compiler::ast` | `src/compiler/ast.hpp` | `crates/lua_compiler/src/ast/` | 抽象语法树。 |
| CodeGenerator | `CodeGenerator` | `lua_compiler::codegen::CodeGenerator` | `src/compiler/codegen/codegen.hpp` | `crates/lua_compiler/src/codegen/mod.rs` | 字节码生成器。 |
| LuaState | `LuaState` | `lua_vm::state::LuaState` | `src/vm/state/lua_state.hpp` | `crates/lua_vm/src/state/lua_state.rs` | 单个 Lua 线程/协程的执行状态。 |
| GlobalState | `GlobalState` | `lua_vm::state::GlobalState` | `src/vm/state/global_state.hpp` | `crates/lua_vm/src/state/global.rs` | 共享运行时状态。 |
| Stack | `Stack` | `lua_vm::state::Stack` | `src/vm/state/stack.hpp` | `crates/lua_vm/src/state/stack.rs` | VM 执行时的值栈。 |
| CallInfo | `CallInfo` | `lua_vm::state::CallInfo` | `src/vm/state/call_info.hpp` | `crates/lua_vm/src/state/call_info.rs` | 调用帧信息。 |
| VM | `Lua::VM` 自由函数 | `lua_vm::execute` | `src/vm/vm.hpp` | `crates/lua_vm/src/execute.rs` | 字节码解释器。 |
| LibModule | `LibModule` catalog | `lua_stdlib::catalog` | `src/lib/lib_catalog.hpp` | `crates/lua_stdlib/src/catalog.rs` | 标准库表驱动注册入口。 |
| RuntimeServices | `RuntimeServices` | `lua_vm::RuntimeServices` | `src/runtime/runtime_services.hpp` | TBD | 显式运行时服务边界。 |

## Naming Conventions

| C++ | Rust |
|---|---|
| `CamelCase` type | `CamelCase` type (struct/enum/trait) |
| `camelCase` method | `snake_case` method |
| `UPPER_CASE` constant | `UPPER_CASE` const |
| `Lua::X::Y` namespace | `crate::x::y` module |
| `std::variant<T...>` | `enum { Variant1(T1), ... }` |
| `std::optional<T>` | `Option<T>` |
| `std::expected<T,E>` | `Result<T, E>` |

## 推荐交叉阅读

- 编译管线：`lua_cpp/docs/compiler/bytecode-generation.md`
- 迁移路线图：`docs/roadmap/rust-migration-workflow.md`
- 类型映射速查：`docs/rust_migration/type_mapping_table.md`
- 偏差日志：`docs/rust_migration/deviation_log.md`

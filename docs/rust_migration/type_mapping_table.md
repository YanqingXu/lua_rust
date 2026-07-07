---
status: current
last_updated: 2026-07-07
applies_to: Rust internal type reference for the Lua interpreter
---

# Rust Type Reference

这份速查表记录项目内部主要 Rust 类型、模块职责和常见建模方式。

## Primitive Types

| 名称 | Rust 类型 | 说明 |
|---|---|---|
| `LuaNumber` | `f64` | Lua 5.1 数字类型。 |
| `LuaInteger` | `i64` | 项目内部整数辅助类型。 |
| `LuByte` | `u8` | GC 标记位、opcode 元数据等使用的字节类型。 |
| `Instruction` | `u32` wrapper | 32-bit Lua 5.1 指令编码。 |
| Light userdata | `GcRef<std::ffi::c_void>` | 不受 GC 管理的宿主指针。 |

## Rust Modeling Patterns

| 场景 | Rust 表达 | 说明 |
|---|---|---|
| 动态值 | `enum` | `Value`、`Expr`、`Stmt` 都使用枚举表达封闭变体集合。 |
| 可选字段 | `Option<T>` | 例如函数原型、元表、可选返回值数量。 |
| 错误传播 | `Result<T, E>` | 编译器错误和运行时错误都显式返回。 |
| GC 引用 | `GcRef<T>` | 对 GC 管理对象的轻量句柄。 |
| 宿主函数 | `fn(...) -> Result<..., ...>` | 标准库函数通过统一调用边界接入 VM。 |
| 共享可变运行时 | 显式状态对象 | `LuaState`、`GarbageCollector` 等由调用方显式传递。 |

## Runtime Core

| 类型/模块 | 位置 | 说明 |
|---|---|---|
| `Value` | `crates/lua_core/src/value.rs` | Lua 值统一表示。 |
| `ValueType` | `crates/lua_core/src/types.rs` | Lua 值类型标签。 |
| `GcObjectType` | `crates/lua_core/src/types.rs` | GC 管理对象类型标签。 |
| `GcObjectHeader` | `crates/lua_core/src/gc/header.rs` | 侵入式 GC 链表节点和标记位。 |
| `GcObject` | `crates/lua_core/src/gc/gc_object.rs` | GC 管理对象 trait。 |
| `GarbageCollector` | `crates/lua_core/src/gc/collector.rs` | GC 分配、标记、清扫入口。 |
| `StringPool` | `crates/lua_core/src/string_pool.rs` | 字符串驻留池。 |
| `Table` | `crates/lua_core/src/table.rs` | Lua 表，包含数组/哈希部分和元表。 |
| `Function` | `crates/lua_core/src/function.rs` | Lua/C 函数闭包。 |
| `Proto` | `crates/lua_core/src/proto.rs` | 函数原型、字节码、常量和调试信息。 |
| `Upvalue` | `crates/lua_core/src/upvalue.rs` | 闭包捕获变量。 |
| `Thread` | `crates/lua_core/src/thread.rs` | Coroutine 对象。 |
| `Userdata` | `crates/lua_core/src/userdata.rs` | 完整用户数据。 |

## Compiler

| 类型/模块 | 位置 | 说明 |
|---|---|---|
| `Token` / `TokenKind` | `crates/lua_compiler/src/token.rs` | Token 类型、值和源码位置。 |
| `Lexer` | `crates/lua_compiler/src/lexer.rs` | 词法分析器。 |
| `Expr` / `Stmt` | `crates/lua_compiler/src/ast/` | 表达式和语句 AST。 |
| `Parser` | `crates/lua_compiler/src/parser/mod.rs` | 递归下降 parser。 |
| `OpCode` | `crates/lua_compiler/src/opcode.rs` | 38 条 Lua 5.1 opcode。 |
| `CodeGenerator` | `crates/lua_compiler/src/codegen/mod.rs` | AST 到 `Proto` 的编译入口。 |
| `RegisterAllocator` | `crates/lua_compiler/src/codegen/reg_alloc.rs` | 寄存器分配。 |
| `BytecodeBuilder` | `crates/lua_compiler/src/codegen/builder.rs` | 指令、常量和调试信息构造器。 |

## VM And Libraries

| 类型/模块 | 位置 | 说明 |
|---|---|---|
| `LuaState` | `crates/lua_vm/src/state/lua_state.rs` | 执行状态、栈、调用帧和全局表。 |
| `Stack` | `crates/lua_vm/src/state/stack.rs` | VM 值栈。 |
| `CallInfo` | `crates/lua_vm/src/state/call_info.rs` | 调用帧信息。 |
| `execute_proto` | `crates/lua_vm/src/execute.rs` | 字节码执行入口。 |
| `RuntimeError` | `crates/lua_vm/src/execute.rs` | VM 运行时错误。 |
| `catalog` | `crates/lua_stdlib/src/catalog.rs` | 标准库注册目录。 |
| `base` / `math` / `string` / `table` | `crates/lua_stdlib/src/` | 常用标准库实现。 |
| `lua_app` | `crates/lua_app/src/main.rs` | 命令行 runner 和 REPL。 |
| `lua_bytecode` | `crates/lua_bytecode/src/main.rs` | 字节码查看工具。 |

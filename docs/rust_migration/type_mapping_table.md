---
status: initial
last_updated: 2026-06-13
applies_to: C++23 → Rust type mapping reference for the Lua interpreter migration
---

# C++ → Rust Type Mapping Table

速查表，记录 C++ 类型到 Rust 类型的映射关系。Phase 推进时增量更新。

## Primitive Types

| C++ Type | Rust Type | Notes |
|---|---|---|
| `void` | `()` | Unit type |
| `bool` | `bool` | Direct mapping |
| `int` / `int32_t` | `i32` | Lua integers are i32 |
| `unsigned int` / `uint32_t` | `u32` | |
| `size_t` | `usize` | |
| `double` / `LuaNumber` | `f64` | Lua uses f64 for numbers |
| `uint8_t` / `lu_byte` | `u8` | Byte type |
| `const char*` | `&std::ffi::CStr` / `*const c_char` | C string boundary |
| `void*` | `*mut std::ffi::c_void` | LightUserdata |

## Standard Library Equivalents

| C++ | Rust | Notes |
|---|---|---|
| `std::string` | `String` | Owned string |
| `std::string_view` | `&str` | Borrowed string slice |
| `std::vector<T>` | `Vec<T>` | Dynamic array |
| `std::unordered_map<K,V>` | `HashMap<K,V>` | Hash table (not ordered) |
| `std::optional<T>` | `Option<T>` | Optional value |
| `std::expected<T,E>` | `Result<T,E>` | Error handling |
| `std::variant<T...>` | `enum { Variant(T)... }` | Tagged union |
| `std::unique_ptr<T>` | `Box<T>` | Unique ownership |
| `std::shared_ptr<T>` | `Arc<T>` | Shared ownership (use sparingly) |
| `std::function<F>` | `Box<dyn Fn(...)>` | Type-erased callable |

## Value System

| C++ | Rust | Phase |
|---|---|---|
| `ValueVariant` (std::variant) | `Value` (enum) | 1.1 |
| `ValueType` (enum) | `ValueType` (#[repr(u8)] enum) | 1.1 |
| `LuaNumber` (double) | `f64` | 1.1 |
| `GCString*` | `GcRef<GcString>` | 1.1 |
| `Table*` | `GcRef<Table>` | 1.1 |
| `Function*` | `GcRef<Function>` | 1.1 |
| `Userdata*` | `GcRef<Userdata>` | 1.1 |
| `Thread*` | `GcRef<Thread>` | 1.1 |

## GC Infrastructure

| C++ | Rust | Phase |
|---|---|---|
| `GCObject` (base class) | `GcObject` (unsafe trait) | 1.2 |
| `GCObject::allObjects_` (intrusive list) | `GcObjectHeader` (#[repr(C)]) | 1.2 |
| `GarbageCollector` | `GarbageCollector` (struct) | 1.3 |
| `GCStrategy` (virtual base) | `GcStrategy` (trait or enum dispatch) | 1.3 |
| `MarkSweepGC` | `MarkSweepGc` | 1.3 |
| `IncrementalGC` | `IncrementalGc` | 1.3 |
| `GCString` | `GcString` | 1.2 |
| `StringPool` | `StringPool` | 1.2 |

## Core Objects

| C++ | Rust | Phase |
|---|---|---|
| `Table` | `Table` | 1.4 |
| `Function` | `Function` (enum: Lua + C) | 1.4 |
| `Proto` | `Proto` | 1.4 |
| `Upvalue` | `Upvalue` | 1.4 |
| `Thread` | `Thread` | 1.4 |
| `Userdata` | `Userdata` | 1.4 |
| `TMS` (tag method enum) | `Tms` | 1.4 |

## Compiler

| C++ | Rust | Phase |
|---|---|---|
| `OpCode` (enum) | `OpCode` (#[repr(u8)] enum) | 2.1 |
| `Instruction` (uint32_t wrapper) | `Instruction` (u32 wrapper) | 2.1 |
| `Token` / `TokenType` | `Token` / `TokenKind` | 2.2 |
| `Lexer` | `Lexer` | 2.2 |
| `Parser` | `Parser` | 2.3 |
| `Expr` / `Stmt` (AST) | `Expr` / `Stmt` | 2.3 |
| `AstVisitor` (template) | `AstVisitor` (trait) | 2.3 |
| `SymbolRef` | `SymbolRef` | 2.4 |
| `ValueResult` | `ValueResult` | 2.4 |
| `CondResult` | `CondResult` | 2.4 |
| `LValueRef` | `LValueRef` | 2.4 |
| `CallResultInfo` | `CallResultInfo` | 2.4 |
| `RegisterAllocator` | `RegisterAllocator` | 2.4 |
| `BytecodeBuilder` | `BytecodeBuilder` | 2.4 |
| `CodeGenerator` | `CodeGenerator` | 2.4 |

## VM

| C++ | Rust | Phase |
|---|---|---|
| `LuaState` | `LuaState` | 3.1 |
| `GlobalState` | `GlobalState` | 3.1 |
| `Stack` | `Stack` | 3.1 |
| `CallInfo` | `CallInfo` | 3.1 |
| `RuntimeServices` | `RuntimeServices` | 3.1 |
| `Lua::VM::execute()` | `Vm::execute_proto()` | 3.2 |
| `execOp*()` handlers | `handler_*()` methods | 3.2 |
| `LuaError` | `RuntimeError` | 3.3 |

## Standard Library

| C++ | Rust | Phase |
|---|---|---|
| `LibModule` / `LibCatalog` | `Catalog` | 4.1 |
| `baselib` / `mathlib` / etc. | `base` / `math` / etc. | 4.1 |
| `lapi.cpp` (C API) | `lua_capi` (FFI) | 4.2 |

## Application

| C++ | Rust | Phase |
|---|---|---|
| `main.cpp` + `repl/*` | `lua_app::main` | 5 |
| `bytecode_main.cpp` + `bytecode_printer.cpp` | `lua_bytecode::main` | 5 |

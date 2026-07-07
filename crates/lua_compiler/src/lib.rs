//! lua_compiler — Lua 5.1 compiler frontend
//!
//! Lexing, parsing, and bytecode code generation for Lua 5.1 sources.
//! Produces `Proto` objects consumable by the `lua_vm` execution engine.
//!
//! ## Module Guide
//! - `token` / `lexer`: Token 类型、源码位置和词法分析。
//! - `ast`: Lua 语法树节点与访问器。
//! - `parser`: 递归下降语法分析器。
//! - `opcode`: Lua 5.1 指令编码、解码和元数据。
//! - `codegen`: 作用域、寄存器分配、跳转回填和 `Proto` 生成。

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

// Phase 2: Compiler frontend
pub mod ast; // ✅ P2.3a — AST node definitions (14 Expr + 13 Stmt variants + Visitor traits)
pub mod codegen; // 🏗️ P2.4 — CodeGen framework (types, reg_alloc, builder; emitters pending)
pub mod lexer; // ✅ P2.1 — Lexer: tokenizer with keyword table, comments, strings, numbers
pub mod opcode; // ✅ P2.1 — OpCode enum, Instruction encode/decode, metadata table
pub mod parser; // ✅ P2.3b — Recursive-descent parser (expr chain, statements, functions, tables)
pub mod token; // ✅ P2.1 — Token types, values, and source positions

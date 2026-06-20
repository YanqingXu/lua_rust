//! lua_compiler — Lua 5.1 compiler frontend
//!
//! Lexing, parsing, and bytecode code generation for Lua 5.1 sources.
//! Produces `Proto` objects consumable by the `lua_vm` execution engine.
//!
//! ## Migration Status
//! - Phase 2 target crate
//! - C++ reference: `lua_cpp/src/compiler/`
//!
//! ## Module Map (C++ → Rust)
//! | C++ | Rust module | Status |
//! |---|---|---|
//! | `src/compiler/opcode.hpp/.cpp` | `opcode` | ✅ P2.1 |
//! | `src/compiler/lexer/*` | `lexer`, `token` | ✅ P2.1 |
//! | `src/compiler/parser/*` | `parser` | pending |
//! | `src/compiler/ast.hpp/.cpp` | `ast` | pending |
//! | `src/compiler/ast_visitor.hpp` | `ast::visitor` | pending |
//! | `src/compiler/codegen/*` | `codegen` | pending |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

// Phase 2: Compiler frontend
pub mod lexer; // ✅ P2.1 — Lexer: tokenizer with keyword table, comments, strings, numbers
pub mod opcode; // ✅ P2.1 — OpCode enum, Instruction encode/decode, metadata table
pub mod token; // ✅ P2.1 — Token types, values, and source positions
// pub mod parser;
// pub mod ast;
// pub mod codegen;

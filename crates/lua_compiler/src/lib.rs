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
//! | `src/compiler/opcode.hpp/.cpp` | `opcode` | pending |
//! | `src/compiler/lexer/*` | `lexer` | pending |
//! | `src/compiler/parser/*` | `parser` | pending |
//! | `src/compiler/ast.hpp/.cpp` | `ast` | pending |
//! | `src/compiler/ast_visitor.hpp` | `ast::visitor` | pending |
//! | `src/compiler/codegen/*` | `codegen` | pending |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

// Public modules — populated during Phase 2
// pub mod opcode;
// pub mod lexer;
// pub mod parser;
// pub mod ast;
// pub mod codegen;

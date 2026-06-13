//! lua_vm — Lua 5.1 virtual machine
//!
//! Register-based bytecode VM with 38 opcode handlers, call frame
//! management, and execution tracing. Preserves Lua 5.1 VM semantics.
//!
//! ## Migration Status
//! - Phase 3 target crate
//! - C++ reference: `lua_cpp/src/vm/`
//!
//! ## Module Map (C++ → Rust)
//! | C++ | Rust module | Status |
//! |---|---|---|
//! | `src/vm/state/*` | `state` | pending |
//! | `src/vm/vm.cpp` + handlers | `execute` | pending |
//! | `src/vm/vm_ops.cpp` | `ops` | pending |
//! | `src/vm/vm_call.cpp` | `call` | pending |
//! | `src/vm/vm_table.cpp` | `table_helpers` | pending |
//! | `src/vm/vm_frame.cpp` | `frame` | pending |
//! | `src/vm/vm_loop.cpp` | `loop_helpers` | pending |
//! | `src/vm/vm_trace.cpp` | `trace` | pending |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

// Public modules — populated during Phase 3
// pub mod state;
// pub mod execute;
// pub mod ops;
// pub mod call;
// pub mod table_helpers;
// pub mod frame;
// pub mod loop_helpers;
// pub mod trace;

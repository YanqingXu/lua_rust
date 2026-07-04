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
//! | `src/vm/state/*` | `state` | ✅ P3.1 |
//! | `src/vm/vm.cpp` + handlers | `execute` | ✅ P3.2 |
//! | `src/vm/vm_ops.cpp` | `execute` (helpers) | ✅ P3.2 |
//! | `src/vm/vm_call.cpp` | `execute` | 🏗️ |
//! | `src/vm/vm_table.cpp` | `execute` (TODO) | 🏗️ |
//! | `src/vm/vm_loop.cpp` | `execute` | ✅ basic |
//! | `src/vm/vm_trace.cpp` | `trace` | pending |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod execute;
pub mod state;

pub use execute::{
    ExecResult, RuntimeError, execute_proto, resume_lua_thread, start_lua_call_at_stack,
};
pub use state::{CallInfo, LUA_MULTRET, LuaState, Stack, ThreadStatus};

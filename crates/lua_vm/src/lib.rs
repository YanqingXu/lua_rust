//! lua_vm — Lua 5.1 virtual machine
//!
//! Register-based bytecode VM with 38 opcode handlers, call frame
//! management, and execution tracing. Preserves Lua 5.1 VM semantics.
//!
//! ## Module Guide
//! - `state`: Lua 栈、调用帧和线程状态。
//! - `execute`: opcode dispatch、调用/返回、元方法、闭包和 coroutine 执行。

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod execute;
pub mod state;

pub use execute::{
    ExecResult, RuntimeError, execute_proto, resume_lua_thread, start_lua_call_at_stack,
};
pub use state::{CallInfo, LUA_MULTRET, LuaState, Stack, ThreadStatus};

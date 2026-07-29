//! VM 状态模块
//!
//! 定义 Lua 虚拟机核心运行时状态类型。
//!

pub mod call_info;
pub mod lua_state;
pub mod stack;
mod vm_context;

pub use call_info::{CallInfo, LUA_MULTRET};
pub use lua_state::{LuaState, ThreadStatus};
pub use stack::Stack;
pub(crate) use vm_context::{ActiveVmContext, enter_vm_context};
pub use vm_context::{with_vm_context, with_vm_context_parts};

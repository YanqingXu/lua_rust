//! VM 状态模块
//!
//! 定义 Lua 虚拟机核心运行时状态类型。
//!
//! C++ 参考: `lua_cpp/src/vm/state/`

pub mod call_info;
pub mod lua_state;
pub mod stack;

pub use call_info::{CallInfo, LUA_MULTRET};
pub use lua_state::{LuaState, ThreadStatus};
pub use stack::Stack;

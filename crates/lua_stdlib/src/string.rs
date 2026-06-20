//! 字符串库 (String Library)
//!
//! Lua 5.1 字符串函数。C++ 参考: `lua_cpp/src/lib/stringlib.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_vm::state::LuaState;

pub fn open_string(_l: &mut LuaState, _gc: &mut GarbageCollector) {
    // String library functions are pending implementation.
    // The library table is created by open_library in catalog.rs.
}

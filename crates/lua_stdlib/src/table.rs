//! 表库 (Table Library)
//!
//! Lua 5.1 表操作函数。C++ 参考: `lua_cpp/src/lib/tablelib.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_vm::state::LuaState;

pub fn open_table(_l: &mut LuaState, _gc: &mut GarbageCollector) {
    // Table library functions are pending implementation.
    // The library table is created by open_library in catalog.rs.
}

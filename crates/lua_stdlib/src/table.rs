//! 表库 (Table Library)
//!
//! Lua 5.1 表操作函数: insert, remove, sort, concat, maxn
//!
//! C++ 参考: `lua_cpp/src/lib/tablelib.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_core::value::Value;
use lua_vm::state::LuaState;

use crate::catalog::register_function;

pub fn open_table(l: &mut LuaState, _gc: &mut GarbageCollector) {
    register_function(l, "insert", lua_tbl_insert);
    register_function(l, "remove", lua_tbl_remove);
    register_function(l, "sort", lua_tbl_sort);
    register_function(l, "concat", lua_tbl_concat);
    register_function(l, "maxn", lua_tbl_maxn);
}

pub fn lua_tbl_insert(_l: &mut LuaState) -> i32 {
    // TODO: table.insert(t, [pos,] value)
    0
}

pub fn lua_tbl_remove(_l: &mut LuaState) -> i32 {
    // TODO: table.remove(t [, pos])
    0
}

pub fn lua_tbl_sort(_l: &mut LuaState) -> i32 {
    // TODO: table.sort(t [, comp])
    0
}

pub fn lua_tbl_concat(l: &mut LuaState) -> i32 {
    let sep = l.stack.at(1).map_or("".to_string(), |v| match v {
        Value::String(_) => "".to_string(),
        Value::Nil => "".to_string(),
        _ => "".to_string(),
    });
    let _i = l.stack.at(2).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    });
    let _j = l.stack.at(3).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    });
    for _ in 0..4 {
        l.pop();
    }
    l.push_value(Value::Nil); // TODO: actual concat
    let _ = sep;
    1
}

pub fn lua_tbl_maxn(l: &mut LuaState) -> i32 {
    l.pop(); // pop table
    l.push_value(Value::Number(0.0)); // TODO: actual maxn
    1
}

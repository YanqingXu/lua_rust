//! 基础库 (Base Library)
#![allow(unused_variables, unused_assignments, clippy::collapsible_if)] // TODO stubs
//!
//! Lua 5.1 核心函数: print, type, tostring, tonumber, error, assert,
//! setmetatable, getmetatable, rawget, rawset, rawequal, select,
//! pcall, xpcall, next, pairs, ipairs, loadstring, dofile, collectgarbage 等。
//!
//! C++ 参考: `lua_cpp/src/lib/baselib.hpp/.cpp`

use lua_core::value::Value;
use lua_vm::state::LuaState;

use crate::catalog::register_function;

/// 打开基础库（注册到全局表 _G）
pub fn open_base(l: &mut LuaState) {
    register_function(l, "print", lua_b_print);
    register_function(l, "type", lua_b_type);
    register_function(l, "tostring", lua_b_tostring);
    register_function(l, "tonumber", lua_b_tonumber);
    register_function(l, "error", lua_b_error);
    register_function(l, "assert", lua_b_assert);
    register_function(l, "setmetatable", lua_b_setmetatable);
    register_function(l, "getmetatable", lua_b_getmetatable);
    register_function(l, "rawget", lua_b_rawget);
    register_function(l, "rawset", lua_b_rawset);
    register_function(l, "rawequal", lua_b_rawequal);
    register_function(l, "select", lua_b_select);
    register_function(l, "pcall", lua_b_pcall);
    register_function(l, "xpcall", lua_b_xpcall);
    register_function(l, "next", lua_b_next);
    register_function(l, "pairs", lua_b_pairs);
    register_function(l, "ipairs", lua_b_ipairs);
    register_function(l, "loadstring", lua_b_loadstring);
    register_function(l, "dofile", lua_b_dofile);
    register_function(l, "collectgarbage", lua_b_collectgarbage);
}

// ═══════════════════════════════════════════════════════════════════
// 基础库函数实现
// ═══════════════════════════════════════════════════════════════════

/// print(...) — 打印所有参数到标准输出
pub fn lua_b_print(l: &mut LuaState) -> i32 {
    let n = l.get_top();
    let mut parts = Vec::new();
    for i in 1..=n {
        let val = l.stack.at((i - 1) as usize).cloned().unwrap_or(Value::Nil);
        parts.push(value_to_string(&val));
    }
    println!("{}", parts.join("\t"));
    0
}

/// type(v) — 返回值的类型名
pub fn lua_b_type(l: &mut LuaState) -> i32 {
    let v = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    let type_name = match v {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::Userdata(_) => "userdata",
        Value::Thread(_) => "thread",
        Value::LightUserdata(_) => "lightuserdata",
    };
    l.pop(); // remove original value
    l.push_value(Value::Nil); // TODO: push GC string
    println!("[type] => {}", type_name);
    1
}

/// tostring(v) — 将值转为字符串
pub fn lua_b_tostring(l: &mut LuaState) -> i32 {
    let v = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    let s = value_to_string(&v);
    l.pop(); // remove original value
    l.push_value(Value::Nil); // TODO: push GC string
    println!("[tostring] => {}", s);
    1
}

/// tonumber(e [, base]) — 将值转为数字
pub fn lua_b_tonumber(l: &mut LuaState) -> i32 {
    let v = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    let mut base: i32 = 10;
    if l.get_top() >= 2 {
        if let Some(Value::Number(n)) = l.stack.at(1) {
            base = *n as i32;
        }
    }
    let result = match &v {
        Value::Number(n) => Some(*n),
        Value::String(_) => None, // TODO: string to number parsing
        _ => None,
    };
    l.pop(); // remove original value
    if l.get_top() > 0 {
        l.pop(); // remove base if present
    }
    match result {
        Some(n) => {
            l.push_value(Value::Number(n));
            1
        }
        None => {
            l.push_value(Value::Nil);
            1
        }
    }
}

/// error(message [, level]) — 抛出 Lua 错误
pub fn lua_b_error(l: &mut LuaState) -> i32 {
    let msg = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    let error_text = value_to_string(&msg);
    panic!("[lua error] {}", error_text);
}

/// assert(v [, message]) — 断言
pub fn lua_b_assert(l: &mut LuaState) -> i32 {
    let v = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    if v.is_false() {
        let msg = if l.get_top() >= 2 {
            value_to_string(&l.stack.at(1).cloned().unwrap_or(Value::Nil))
        } else {
            "assertion failed!".to_string()
        };
        panic!("[lua assert] {}", msg);
    }
    // 返回所有参数
    l.get_top()
}

/// setmetatable(table, metatable) — 设置元表
pub fn lua_b_setmetatable(_l: &mut LuaState) -> i32 {
    // TODO: Table::set_metatable()
    1
}

/// getmetatable(object) — 获取元表
pub fn lua_b_getmetatable(l: &mut LuaState) -> i32 {
    l.push_value(Value::Nil); // TODO: actual metatable lookup
    1
}

/// rawget(table, index) — 绕过元方法获取
pub fn lua_b_rawget(_l: &mut LuaState) -> i32 {
    // TODO: Table::raw_get()
    1
}

/// rawset(table, index, value) — 绕过元方法设置
pub fn lua_b_rawset(_l: &mut LuaState) -> i32 {
    // TODO: Table::raw_set()
    1
}

/// rawequal(v1, v2) — 绕过元方法比较
pub fn lua_b_rawequal(l: &mut LuaState) -> i32 {
    let v1 = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    let v2 = l.stack.at(1).cloned().unwrap_or(Value::Nil);
    l.pop();
    l.pop();
    l.push_value(Value::Boolean(v1 == v2));
    1
}

/// select(index, ...) — 选择参数
pub fn lua_b_select(l: &mut LuaState) -> i32 {
    let n = l.get_top();
    let idx = l.stack.at(0).cloned().unwrap_or(Value::Nil);
    if let Value::String(s) = &idx {
        if false {
            let _ = s; // "#" — return count
        }
        // TODO: check for "#" string
    }
    if let Value::Number(n_idx) = &idx {
        let i = *n_idx as i32;
        if i >= 1 && i <= n {
            return n - i + 1;
        }
    }
    l.push_value(Value::Nil);
    1
}

/// pcall(f, ...) — 保护模式调用
pub fn lua_b_pcall(l: &mut LuaState) -> i32 {
    // TODO: protected call via LuaState::pcall
    l.push_value(Value::Boolean(true));
    1
}

/// xpcall(f, msgh, ...) — 带错误处理器的保护调用
pub fn lua_b_xpcall(l: &mut LuaState) -> i32 {
    // TODO: protected call with error handler
    l.push_value(Value::Boolean(true));
    1
}

/// next(table [, index]) — 遍历表
pub fn lua_b_next(_l: &mut LuaState) -> i32 {
    // TODO: Table::next()
    1
}

/// pairs(t) — 创建通用迭代器
pub fn lua_b_pairs(l: &mut LuaState) -> i32 {
    // TODO: return next, t, nil
    l.push_value(Value::Nil); // next function
    l.push_value(l.stack.at(0).cloned().unwrap_or(Value::Nil)); // table
    l.push_value(Value::Nil); // initial index
    3
}

/// ipairs(t) — 创建数组迭代器
pub fn lua_b_ipairs(l: &mut LuaState) -> i32 {
    // TODO: return ipairs_iter, t, 0
    l.push_value(Value::Nil); // ipairs_iter function
    l.push_value(l.stack.at(0).cloned().unwrap_or(Value::Nil)); // table
    l.push_value(Value::Number(0.0)); // initial index
    3
}

/// loadstring(string [, chunkname]) — 编译字符串
pub fn lua_b_loadstring(l: &mut LuaState) -> i32 {
    // TODO: compile string via lua_compiler
    l.push_value(Value::Nil); // function (nil = error)
    1
}

/// loadfile([filename]) — 编译文件
pub fn lua_b_loadfile(l: &mut LuaState) -> i32 {
    l.push_value(Value::Nil);
    1
}

/// dofile([filename]) — 执行文件
pub fn lua_b_dofile(l: &mut LuaState) -> i32 {
    l.push_value(Value::Nil);
    1
}

/// collectgarbage(opt [, arg]) — 垃圾回收控制
pub fn lua_b_collectgarbage(l: &mut LuaState) -> i32 {
    l.push_value(Value::Number(0.0)); // TODO: actual GC stats
    1
}

// ═══════════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════════

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => format!("{}", n),
        Value::String(_s) => "string".to_string(),
        Value::Table(_) => "table: 0x...".to_string(),
        Value::Function(_) => "function: 0x...".to_string(),
        Value::Userdata(_) => "userdata: 0x...".to_string(),
        Value::Thread(_) => "thread: 0x...".to_string(),
        Value::LightUserdata(_) => "lightuserdata".to_string(),
    }
}

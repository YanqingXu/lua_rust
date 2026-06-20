//! 字符串库 (String Library)
#![allow(dead_code, clippy::collapsible_if)] // TODO stubs
//!
//! Lua 5.1 字符串函数: sub, upper, lower, len, reverse, byte, char,
//! find, match, gmatch, gsub, format, rep, dump
//!
//! C++ 参考: `lua_cpp/src/lib/stringlib.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_core::value::Value;
use lua_vm::state::LuaState;

use crate::catalog::register_function;

pub fn open_string(l: &mut LuaState, _gc: &mut GarbageCollector) {
    register_function(l, "len", lua_str_len);
    register_function(l, "sub", lua_str_sub);
    register_function(l, "upper", lua_str_upper);
    register_function(l, "lower", lua_str_lower);
    register_function(l, "reverse", lua_str_reverse);
    register_function(l, "byte", lua_str_byte);
    register_function(l, "char", lua_str_char);
    register_function(l, "find", lua_str_find);
    register_function(l, "rep", lua_str_rep);
    register_function(l, "format", lua_str_format);
}

fn get_str(l: &LuaState, idx: usize) -> String {
    l.stack.at(idx).map_or("".to_string(), |v| match v {
        Value::String(_) => "string".to_string(), // TODO
        Value::Number(n) => n.to_string(),
        _ => "".to_string(),
    })
}

fn push_str(l: &mut LuaState, s: &str) {
    l.pop();
    l.push_value(Value::Nil); // TODO: GC string
    let _ = s;
}

pub fn lua_str_len(l: &mut LuaState) -> i32 {
    let s = get_str(l, 0);
    l.pop();
    l.push_value(Value::Number(s.len() as f64));
    1
}

pub fn lua_str_sub(l: &mut LuaState) -> i32 {
    let _s = get_str(l, 0);
    let _start = l
        .stack
        .at(1)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(1);
    let _end = l
        .stack
        .at(2)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(-1);
    l.pop();
    l.pop();
    l.pop();
    l.push_value(Value::Nil); // TODO
    1
}

pub fn lua_str_upper(l: &mut LuaState) -> i32 {
    let s = get_str(l, 0).to_uppercase();
    l.pop();
    l.push_value(Value::Nil); // TODO
    let _ = s;
    1
}

pub fn lua_str_lower(l: &mut LuaState) -> i32 {
    let s = get_str(l, 0).to_lowercase();
    l.pop();
    l.push_value(Value::Nil); // TODO
    let _ = s;
    1
}

pub fn lua_str_reverse(l: &mut LuaState) -> i32 {
    let s: String = get_str(l, 0).chars().rev().collect();
    l.pop();
    l.push_value(Value::Nil); // TODO
    let _ = s;
    1
}

pub fn lua_str_byte(l: &mut LuaState) -> i32 {
    let s = get_str(l, 0);
    let i = l
        .stack
        .at(1)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as usize),
            _ => None,
        })
        .unwrap_or(1);
    let byte = s
        .as_bytes()
        .get(i - 1)
        .copied()
        .map(|b| b as f64)
        .unwrap_or(0.0);
    l.pop();
    l.pop();
    l.push_value(Value::Number(byte));
    1
}

pub fn lua_str_char(l: &mut LuaState) -> i32 {
    let n = l.get_top() as usize;
    let mut result = String::new();
    for i in 0..n {
        if let Some(Value::Number(c)) = l.stack.at(i) {
            if let Some(ch) = char::from_u32(*c as u32) {
                result.push(ch);
            }
        }
    }
    for _ in 0..n {
        l.pop();
    }
    l.push_value(Value::Nil); // TODO: GC string
    let _ = result;
    1
}

pub fn lua_str_find(l: &mut LuaState) -> i32 {
    l.pop();
    l.pop(); // TODO
    l.push_value(Value::Nil);
    1
}

pub fn lua_str_rep(l: &mut LuaState) -> i32 {
    let s = get_str(l, 0);
    let n = l
        .stack
        .at(1)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as usize),
            _ => None,
        })
        .unwrap_or(1);
    let result = s.repeat(n);
    l.pop();
    l.pop();
    l.push_value(Value::Nil); // TODO
    let _ = result;
    1
}

pub fn lua_str_format(l: &mut LuaState) -> i32 {
    let n = l.get_top() as usize;
    for _ in 0..n {
        l.pop();
    }
    l.push_value(Value::Nil); // TODO: format
    1
}

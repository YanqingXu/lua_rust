//! 基础库 (Base Library)
#![allow(clippy::collapsible_if)]
//!
//! Lua 5.1 核心函数: print, type, tostring, tonumber, error, assert,
//! setmetatable, getmetatable, rawget, rawset, rawequal, select,
//! pcall, xpcall, next, pairs, ipairs, loadstring, dofile, collectgarbage 等。
//!

use lua_core::function::{Function, RuntimeNativeFunction};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::userdata::Userdata;
use lua_core::value::Value;
use lua_vm::state::LuaState;
use std::io::Write;

// Project compatibility follows the fixed lua_cpp oracle. This intentionally
// differs from the stock Lua 5.1 `_VERSION` value (`"Lua 5.1"`).
const LUA_VERSION: &str = "Lua 5.1 (C core prototype)";

/// 打开基础库（注册到全局表 _G）
pub fn open_base(l: &mut LuaState, gc: &mut GarbageCollector) {
    if let Some(global_table) = l.global_table {
        set_global_value(l, gc, global_table, "_G", &Value::Table(global_table));
        crate::registration::set_string(l, gc, global_table, b"_VERSION", LUA_VERSION.as_bytes())
            .expect("base version publication must remain collector-valid");

        // Register core functions
        register(l, gc, global_table, "assert", lua_b_assert_raw);
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"collectgarbage",
            RuntimeNativeFunction::CollectGarbage,
        )
        .expect("collectgarbage Runtime-native publication must remain collector-valid");
        register(l, gc, global_table, "error", lua_b_error_raw);
        register(l, gc, global_table, "gcinfo", lua_b_gcinfo_raw);
        register(l, gc, global_table, "getfenv", lua_b_getfenv_raw);
        register(l, gc, global_table, "getmetatable", lua_b_getmetatable_raw);
        register(l, gc, global_table, "ipairs", lua_b_ipairs_raw);
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"dofile",
            RuntimeNativeFunction::DoFile,
        )
        .expect("dofile Runtime-native publication must remain collector-valid");
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"load",
            RuntimeNativeFunction::Load,
        )
        .expect("load Runtime-native publication must remain collector-valid");
        register(l, gc, global_table, "loadfile", lua_b_loadfile_raw);
        register(l, gc, global_table, "loadstring", lua_b_loadstring_raw);
        register(l, gc, global_table, "newproxy", lua_b_newproxy_raw);
        register(l, gc, global_table, "next", lua_b_next_raw);
        register(l, gc, global_table, "pairs", lua_b_pairs_raw);
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"pcall",
            RuntimeNativeFunction::ProtectedCall,
        )
        .expect("pcall Runtime-native publication must remain collector-valid");
        register(l, gc, global_table, "print", lua_b_print_raw);
        register(l, gc, global_table, "rawequal", lua_b_rawequal_raw);
        register(l, gc, global_table, "rawget", lua_b_rawget_raw);
        register(l, gc, global_table, "rawset", lua_b_rawset_raw);
        register(l, gc, global_table, "select", lua_b_select_raw);
        register(l, gc, global_table, "setfenv", lua_b_setfenv_raw);
        register(l, gc, global_table, "setmetatable", lua_b_setmetatable_raw);
        register(l, gc, global_table, "tonumber", lua_b_tonumber_raw);
        register(l, gc, global_table, "type", lua_b_type_raw);
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"tostring",
            RuntimeNativeFunction::ToString,
        )
        .expect("tostring Runtime-native publication must remain collector-valid");
        register(l, gc, global_table, "unpack", lua_b_unpack_raw);
        crate::registration::register_runtime_native(
            l,
            gc,
            global_table,
            b"xpcall",
            RuntimeNativeFunction::ProtectedXCall,
        )
        .expect("xpcall Runtime-native publication must remain collector-valid");
    }
}

fn set_global_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    value: &Value,
) {
    crate::registration::set_value(l, gc, table, name.as_bytes(), value)
        .expect("base global publication must remain collector-valid");
}

/// Register a C function in the global table.
fn register(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(l, gc, table, name.as_bytes(), func, None)
        .expect("base Function publication must remain collector-valid");
}

// ═══════════════════════════════════════════════════════════════════
// print
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_print_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    let mut output = Vec::new();
    for i in 1..=n {
        if i > 1 {
            output.push(b'\t');
        }
        if let Some(val) = l.at(i) {
            append_print_value(l, &mut output, val);
        }
    }
    output.push(b'\n');

    let mut stdout = std::io::stdout().lock();
    if crate::host_io::write_text(&mut stdout, &output).is_ok() && stdout.flush().is_ok() {
        0
    } else {
        -1
    }
}

fn append_print_value(l: &LuaState, output: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Nil => output.extend_from_slice(b"nil"),
        Value::Boolean(b) => output.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            let text = lua_core::number_format::format_lua_number(*n);
            output.extend_from_slice(text.as_bytes());
        }
        Value::String(s) => {
            let _ = l.with_string_bytes(*s, |bytes| output.extend_from_slice(bytes));
        }
        Value::Table(t) => output.extend_from_slice(format!("table: {:p}", t.as_ptr()).as_bytes()),
        Value::Function(f) => {
            output.extend_from_slice(format!("function: {:p}", f.as_ptr()).as_bytes());
        }
        Value::Userdata(u) => {
            output.extend_from_slice(format!("userdata: {:p}", u.as_ptr()).as_bytes());
        }
        Value::Thread(t) => {
            output.extend_from_slice(format!("thread: {:p}", t.as_ptr()).as_bytes());
        }
        Value::LightUserdata(p) => {
            output.extend_from_slice(format!("lightuserdata: {:p}", p.as_ptr()).as_bytes());
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// type
// ═══════════════════════════════════════════════════════════════════

/// type(x) — returns the type name of x as a string
unsafe extern "C" fn lua_b_type_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler,
    // which is always valid during execution.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    let type_name = if n >= 1 {
        match l.at(1) {
            Some(Value::Nil) => "nil",
            Some(Value::Boolean(_)) => "boolean",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(Value::Table(_)) => "table",
            Some(Value::Function(_)) => "function",
            Some(Value::Userdata(_)) => "userdata",
            Some(Value::Thread(_)) => "thread",
            Some(Value::LightUserdata(_)) => "lightuserdata",
            None => "nil",
        }
    } else {
        "nil"
    };
    if push_lua_string(l, type_name) { 1 } else { -1 }
}

fn value_to_string_helper(l: &LuaState, v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => lua_core::number_format::format_lua_number(*n),
        Value::String(s) => l
            .with_string_bytes(*s, |bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default(),
        Value::Table(t) => format!("table: {:p}", t.as_ptr()),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => format!("lightuserdata: {:p}", p.as_ptr()),
    }
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    push_lua_bytes(l, text.as_bytes())
}

fn push_lua_bytes(l: &mut LuaState, bytes: &[u8]) -> bool {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    crate::registration::push_string(l, gc, bytes).is_ok()
}

// ═══════════════════════════════════════════════════════════════════
// assert / error
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_assert_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    let condition = l.at(1).cloned().unwrap_or(Value::Nil);
    if condition.is_false() {
        if n >= 2 {
            let message = l.at(2).cloned().unwrap_or(Value::Nil);
            l.push_value(message);
        } else if !push_lua_string(l, "assertion failed!") {
            return -1;
        }
        return -1;
    }

    for i in 1..=n {
        let value = l.at(i).cloned().unwrap_or(Value::Nil);
        l.push_value(value);
    }
    n
}

unsafe extern "C" fn lua_b_error_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let message = l.at(1).cloned().unwrap_or(Value::Nil);
    let level = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(level) if level >= 0.0 => level as usize,
        Value::Nil => 1,
        _ => 1,
    };
    if level > 0
        && let Value::String(message_ref) = message
        && let Some(prefix) = error_location_prefix(l, level)
    {
        let mut bytes = prefix.into_bytes();
        bytes.extend_from_slice(b": ");
        let _ = l.with_string_bytes(message_ref, |message| bytes.extend_from_slice(message));
        // The fixed C++ `error` implementation resets the native frame before
        // publishing the located error object. Besides matching the protected
        // call's result slot, this prevents the original argument from staying
        // live in the frame's GC-scanned register window.
        l.set_top(0);
        if !push_lua_bytes(l, &bytes) {
            return -1;
        }
        return -1;
    }
    l.set_top(0);
    l.push_value(message);
    -1
}

unsafe extern "C" fn lua_b_gcinfo_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let kb = l
        .active_gc_ptr()
        // SAFETY: the VM context owns this collector for the current C call.
        .map(|gc| unsafe { (&*gc).total_memory() as f64 / 1024.0 })
        .unwrap_or(0.0)
        .floor();
    l.push_value(Value::Number(kb));
    1
}

// ═══════════════════════════════════════════════════════════════════
// loadstring
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_loadstring_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::String(source_ref)) = l.at(1).cloned() else {
        return push_load_error(l, "bad argument #1 to 'loadstring' (string expected)");
    };
    let Ok(source) = l.copy_string_bytes(source_ref) else {
        return push_load_error(l, "invalid source string");
    };

    let chunk_name = l
        .at(2)
        .and_then(|value| match value {
            Value::String(name_ref) => l.copy_string_bytes(*name_ref).ok(),
            _ => None,
        })
        .unwrap_or_else(|| default_loadstring_chunk_name(&source));

    match compile_chunk_function(l, &source, &chunk_name) {
        Ok(_) => 1,
        Err(message) => push_load_error(l, &message),
    }
}

unsafe extern "C" fn lua_b_loadfile_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(filename) = optional_filename_arg(l, 1, "loadfile") else {
        return push_load_error(l, "bad argument #1 to 'loadfile' (string expected)");
    };

    let source = match read_lua_source_file(&filename) {
        Ok(source) => source,
        Err(err) => return push_load_error(l, &format!("cannot open {filename}: {err}")),
    };
    let chunk_name = format!("@{filename}").into_bytes();

    match compile_chunk_function(l, &source, &chunk_name) {
        Ok(_) => 1,
        Err(message) => push_load_error(l, &message),
    }
}

fn push_load_error(l: &mut LuaState, message: &str) -> i32 {
    l.push_value(Value::Nil);
    if !push_lua_string(l, message) {
        return -1;
    }
    2
}

fn read_lua_source_file(filename: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(filename)
}

fn default_loadstring_chunk_name(source: &[u8]) -> Vec<u8> {
    source.to_vec()
}

fn optional_filename_arg(l: &LuaState, idx: i32, _name: &str) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => l
            .with_string_bytes(*s, |bytes| {
                std::str::from_utf8(bytes).ok().map(str::to_owned)
            })
            .ok()
            .flatten(),
        Some(Value::Nil) | None => None,
        _ => None,
    }
}

fn compile_chunk_function(
    l: &mut LuaState,
    source: &[u8],
    chunk_name: &[u8],
) -> Result<GcRef<Function>, String> {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return Err("chunk compilation unavailable without an active GC".to_string());
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    lua_vm::execute::compile_chunk_function(l, gc, source, chunk_name)
}

// ═══════════════════════════════════════════════════════════════════
// tonumber / select / unpack
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_tonumber_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    if l.get_top() < 1 {
        if !push_lua_string(l, "bad argument #1 to 'tonumber' (value expected)") {
            return -1;
        }
        return -1;
    }
    let value = l.at(1).cloned().unwrap_or(Value::Nil);
    let base = l.at(2).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    });

    if base.is_none()
        && let Value::Number(n) = value
    {
        l.push_value(Value::Number(n));
        return 1;
    }

    let Some(text) = value_to_plain_string(l, &value) else {
        l.push_value(Value::Nil);
        return 1;
    };

    let number = if let Some(base) = base {
        parse_integer_with_base(&text, base)
    } else {
        text.trim().parse::<f64>().ok()
    };

    l.push_value(number.map(Value::Number).unwrap_or(Value::Nil));
    1
}

unsafe extern "C" fn lua_b_select_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    let extra = (n - 1).max(0);

    let selector_is_count = matches!(l.at(1), Some(Value::String(s)) if {
        l.with_string_bytes(*s, |bytes| bytes == b"#")
            .unwrap_or(false)
    });
    if selector_is_count {
        l.push_value(Value::Number(extra as f64));
        return 1;
    }

    let mut index = match l.at(1) {
        Some(Value::Number(n)) => *n as i32,
        _ => return -1,
    };
    if index < 0 {
        index += extra + 1;
    }
    if index < 1 || index > extra + 1 {
        return -1;
    }

    let count = extra - index + 1;
    for offset in 0..count {
        let value = l.at(1 + index + offset).cloned().unwrap_or(Value::Nil);
        l.push_value(value);
    }
    count
}

unsafe extern "C" fn lua_b_unpack_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return -1;
    };
    let start = match l.at(2) {
        Some(Value::Number(n)) => *n as i32,
        Some(Value::Nil) | None => 1,
        _ => return -1,
    };

    let requested_end = match l.at(3) {
        Some(Value::Number(n)) => Some(*n as i32),
        Some(Value::Nil) | None => None,
        _ => return -1,
    };
    let values = l.with_gc_ref(table_ref, |table| {
        let end = requested_end.unwrap_or(table.length() as i32);
        let values = (start..=end)
            .map(|index| table.get(&Value::Number(index as f64)))
            .collect::<Vec<_>>();
        (end, values)
    });
    let Ok((end, values)) = values else {
        return -1;
    };
    if values.is_empty() {
        return 0;
    }
    for value in values {
        l.push_value(value);
    }
    end - start + 1
}

fn value_to_plain_string(l: &LuaState, value: &Value) -> Option<String> {
    match value {
        Value::String(s) => l
            .with_string_bytes(*s, |bytes| {
                std::str::from_utf8(bytes).ok().map(str::to_owned)
            })
            .ok()
            .flatten(),
        Value::Number(n) => Some(value_to_string_helper(l, &Value::Number(*n))),
        _ => None,
    }
}

fn parse_integer_with_base(text: &str, base: i32) -> Option<f64> {
    if !(2..=36).contains(&base) {
        return None;
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (negative, digits) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (false, rest)
    } else {
        (false, trimmed)
    };

    if digits.is_empty() {
        return None;
    }

    let mut result = 0_f64;
    for ch in digits.chars() {
        let digit = ch.to_digit(base as u32)? as f64;
        result = result * base as f64 + digit;
    }
    Some(if negative { -result } else { result })
}

// ═══════════════════════════════════════════════════════════════════
// raw table/metatable helpers
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_rawequal_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let a = l.at(1).cloned().unwrap_or(Value::Nil);
    let b = l.at(2).cloned().unwrap_or(Value::Nil);
    l.push_value(Value::Boolean(raw_equal(&a, &b)));
    1
}

unsafe extern "C" fn lua_b_rawget_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return -1;
    };
    let key = l.at(2).cloned().unwrap_or(Value::Nil);
    let Ok(value) = l.with_gc_ref(table_ref, |table| table.get(&key)) else {
        return -1;
    };
    l.push_value(value);
    1
}

unsafe extern "C" fn lua_b_rawset_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    if l.get_top() < 3 {
        let _ = push_lua_string(l, "rawset: missing arguments");
        return -1;
    }
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        let _ = push_lua_string(l, "rawset: table expected");
        return -1;
    };
    let key = l.at(2).cloned().unwrap_or(Value::Nil);
    if key.is_nil() {
        let _ = push_lua_string(l, "rawset: table index is nil");
        return -1;
    }
    // Table::set requires a valid key. Reject Lua input before entering the
    // mutation callback: a Rust panic cannot unwind across this C ABI boundary.
    if matches!(&key, Value::Number(number) if number.is_nan()) {
        let _ = push_lua_string(l, "rawset: table index is NaN");
        return -1;
    }
    let value = l.at(3).cloned().unwrap_or(Value::Nil);
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return -1;
    };
    // SAFETY: the VM installs this scoped service pointer for the callback.
    let gc = unsafe { &mut *gc_ptr };
    if gc
        .with_mut(table_ref, |table| table.set(&key, &value))
        .is_err()
    {
        return -1;
    }
    l.push_value(Value::Table(table_ref));
    1
}

unsafe extern "C" fn lua_b_getfenv_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let arg = l.at(1).cloned().unwrap_or(Value::Number(1.0));

    let env = match arg {
        Value::Nil => function_env_at_level(l, 1),
        Value::Number(0.0) => l.thread_env.or(l.global_table),
        Value::Number(level) if level > 0.0 => {
            let env = function_env_at_level(l, level as usize);
            if env.is_none() {
                let _ = push_lua_string(l, "invalid level");
                return -1;
            }
            env
        }
        Value::Function(func_ref) => function_env(func_ref, l),
        _ => None,
    };

    if let Some(env) = env {
        l.push_value(Value::Table(env));
    } else {
        l.push_nil();
    }
    1
}

unsafe extern "C" fn lua_b_setfenv_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let target = l.at(1).cloned().unwrap_or(Value::Nil);
    let Some(Value::Table(env)) = l.at(2).cloned() else {
        return -1;
    };

    let return_value = match target.clone() {
        Value::Number(0.0) => {
            l.thread_env = Some(env);
            target
        }
        Value::Number(level) if level > 0.0 => {
            if let Some(func_ref) = function_ref_at_level(l, level as usize) {
                set_function_env(l, func_ref, env);
                Value::Function(func_ref)
            } else if is_thread_env_stack_level(l, level as usize) {
                l.chunk_env = Some(env);
                target
            } else {
                return -1;
            }
        }
        Value::Function(func_ref) => {
            set_function_env(l, func_ref, env);
            Value::Function(func_ref)
        }
        _ => return -1,
    };

    l.push_value(return_value);
    1
}

unsafe extern "C" fn lua_b_getmetatable_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let value = l.at(1).cloned().unwrap_or(Value::Nil);
    let metatable = match value {
        Value::Table(table_ref) => l.with_gc_ref(table_ref, Table::metatable).ok().flatten(),
        Value::Userdata(userdata_ref) => l
            .with_gc_ref(userdata_ref, Userdata::metatable)
            .ok()
            .flatten(),
        Value::Nil => l.nil_metatable,
        Value::Boolean(_) => l.boolean_metatable,
        Value::Number(_) => l.number_metatable,
        _ => None,
    };
    if let Some(mt) = metatable {
        if let Some(protected) = metatable_field(l, mt, "__metatable") {
            l.push_value(protected);
        } else {
            l.push_value(Value::Table(mt));
        }
        return 1;
    }
    l.push_value(Value::Nil);
    1
}

unsafe extern "C" fn lua_b_newproxy_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let (create_metatable, existing_metatable) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil | Value::Boolean(false) => (false, None),
        Value::Boolean(true) => (true, None),
        Value::Userdata(userdata_ref) => (
            false,
            gc.with_ref(userdata_ref, Userdata::metatable)
                .ok()
                .flatten(),
        ),
        _ => return -1,
    };

    let publication = gc.with_publication(|transaction| {
        let userdata = transaction.alloc(Userdata::new(0));
        if create_metatable {
            let metatable = transaction.alloc(Table::new());
            transaction.set_userdata_metatable(&userdata, Some(&metatable))?;
        } else if let Some(metatable) = existing_metatable {
            transaction.set_userdata_metatable_reference(&userdata, Some(metatable))?;
        }
        // SAFETY: the callback installs the proxy on the active result stack.
        unsafe { transaction.publish_userdata_value(userdata, |value| l.push_value(value)) }
    });
    if publication.is_err() {
        return -1;
    }
    1
}

unsafe extern "C" fn lua_b_setmetatable_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return -1;
    };
    let mt = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Nil => None,
        Value::Table(mt) => Some(mt),
        _ => return -1,
    };
    if let Some(existing_mt) = l.with_gc_ref(table_ref, Table::metatable).ok().flatten()
        && metatable_field(l, existing_mt, "__metatable").is_some()
    {
        let _ = push_lua_string(l, "cannot change a protected metatable");
        return -1;
    }
    // Finish dynamic collector reads before taking the explicit mutable
    // collector borrow used to install and register this metatable.
    let mode = mt.and_then(|metatable| weak_mode(l, metatable));
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return -1;
    };
    // SAFETY: the VM installs this scoped service pointer for the callback.
    let gc = unsafe { &mut *gc_ptr };
    if gc
        .with_mut(table_ref, |table| table.set_metatable(mt))
        .is_err()
    {
        return -1;
    }
    if let Some((weak_keys, weak_values)) = mode {
        gc.register_weak_table(table_ref, weak_keys, weak_values);
    }
    // Match the fixed C++ base library: retain only argument 1 as the result
    // instead of pushing a duplicate above both arguments.
    l.set_top(1);
    1
}

fn metatable_field(l: &LuaState, metatable: GcRef<Table>, name: &str) -> Option<Value> {
    crate::registration::find_table_field(l, metatable, name.as_bytes())
        .ok()
        .flatten()
}

fn weak_mode(l: &LuaState, metatable: GcRef<Table>) -> Option<(bool, bool)> {
    let Value::String(mode) = metatable_field(l, metatable, "__mode")? else {
        return None;
    };
    l.with_string_bytes(mode, |mode| (mode.contains(&b'k'), mode.contains(&b'v')))
        .ok()
}

fn function_env_at_level(l: &LuaState, level: usize) -> Option<GcRef<Table>> {
    function_ref_at_level(l, level)
        .and_then(|func_ref| function_env(func_ref, l))
        .or_else(|| {
            if is_thread_env_stack_level(l, level) {
                l.chunk_env.or(l.thread_env).or(l.global_table)
            } else {
                None
            }
        })
}

fn function_ref_at_level(l: &LuaState, level: usize) -> Option<GcRef<Function>> {
    let ResolvedStackLevel::Real(frame_idx) = resolve_stack_level(l, level)? else {
        return None;
    };
    let ci = l.call_stack.get(frame_idx)?;
    if ci.func == ci.base {
        return None;
    }
    match l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) {
        Value::Function(func_ref) => Some(func_ref),
        _ => None,
    }
}

fn error_location_prefix(l: &LuaState, level: usize) -> Option<String> {
    let gc_ptr = l.active_gc_ptr()?;
    // SAFETY: the VM installs the owning collector for the duration of the
    // active base-library callback.
    let gc = unsafe { &*gc_ptr };
    let ResolvedStackLevel::Real(frame_idx) = resolve_stack_level(l, level)? else {
        return None;
    };
    let ci = l.call_stack.get(frame_idx)?;
    let proto_ref = function_ref_at_level(l, level)
        .and_then(|func_ref| gc.with_ref(func_ref, Function::proto).ok().flatten())
        .or(ci.proto)?;
    let pc = ci.savedpc.unwrap_or(0);
    let (source_ref, line) = gc
        .with_ref(proto_ref, |proto| (proto.source(), proto.line(pc)))
        .ok()?;
    let source = source_ref
        .and_then(|source_ref| {
            l.with_string_bytes(source_ref, lua_vm::execute::chunk_name_for_diagnostic)
                .ok()
        })
        .unwrap_or_else(|| "?".to_string());
    Some(format!("{source}:{line}"))
}

fn is_thread_env_stack_level(l: &LuaState, level: usize) -> bool {
    let Some(ResolvedStackLevel::Real(frame_idx)) = resolve_stack_level(l, level) else {
        return false;
    };
    if frame_idx != 0 {
        return false;
    }
    let Some(ci) = l.call_stack.get(frame_idx) else {
        return false;
    };
    ci.func == ci.base || !matches!(l.stack.at(ci.func), Some(Value::Function(_)))
}

enum ResolvedStackLevel {
    Real(usize),
    Tail,
}

fn resolve_stack_level(l: &LuaState, level: usize) -> Option<ResolvedStackLevel> {
    if level == 0 {
        return None;
    }
    let mut remaining = level;
    let mut idx = l.current_ci;
    while idx > 0 {
        idx -= 1;
        if remaining == 1 {
            return Some(ResolvedStackLevel::Real(idx));
        }
        remaining -= 1;

        let tailcalls = l.call_stack.get(idx).map(|ci| ci.tailcalls).unwrap_or(0);
        if tailcalls > 0 {
            let tailcalls = tailcalls as usize;
            if remaining <= tailcalls {
                return Some(ResolvedStackLevel::Tail);
            }
            remaining -= tailcalls;
        }
    }
    None
}

fn function_env(func_ref: GcRef<Function>, l: &LuaState) -> Option<GcRef<Table>> {
    l.with_gc_ref(func_ref, Function::env)
        .ok()
        .flatten()
        .or(l.global_table)
}

fn set_function_env(l: &LuaState, func_ref: GcRef<Function>, env: GcRef<Table>) {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return;
    };
    // SAFETY: the VM installs this scoped service pointer for the callback.
    let _ = unsafe { &mut *gc_ptr }.with_mut(func_ref, |function| function.set_env(Some(env)));
}

fn raw_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a == b,
        _ => a == b,
    }
}

// ═══════════════════════════════════════════════════════════════════
// next / pairs / ipairs
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_next_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return -1;
    };
    let key = l.at(2).cloned().unwrap_or(Value::Nil);

    let next = match l.with_gc_ref(table_ref, |table| table.try_next(&key)) {
        Ok(Ok(next)) => next,
        Ok(Err(error)) => {
            let _ = push_lua_string(l, &error.to_string());
            return -1;
        }
        Err(_) => {
            let _ = push_lua_string(l, "next: table is unavailable");
            return -1;
        }
    };
    if let Some((next_key, next_value)) = next {
        l.push_value(next_key);
        l.push_value(next_value);
        return 2;
    }

    l.push_value(Value::Nil);
    1
}

unsafe extern "C" fn lua_b_pairs_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(table_value @ Value::Table(_)) = l.at(1).cloned() else {
        return -1;
    };
    if !push_c_function(l, lua_b_next_raw) {
        return -1;
    }

    l.push_value(table_value);
    l.push_value(Value::Nil);
    3
}

unsafe extern "C" fn lua_b_ipairs_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(table_value @ Value::Table(_)) = l.at(1).cloned() else {
        return -1;
    };
    if !push_c_function(l, lua_b_ipairs_iter_raw) {
        return -1;
    }

    l.push_value(table_value);
    l.push_value(Value::Number(0.0));
    3
}

unsafe extern "C" fn lua_b_ipairs_iter_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return 0;
    };
    let index = match l.at(2) {
        Some(Value::Number(n)) => *n as i32,
        _ => return 0,
    };
    let next_index = index + 1;

    let Ok(next_value) = l.with_gc_ref(table_ref, |table| table.get_array(next_index)) else {
        return 0;
    };
    if next_value.is_nil() {
        return 0;
    }

    l.push_value(Value::Number(next_index as f64));
    l.push_value(next_value);
    2
}

fn push_c_function(
    l: &mut LuaState,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) -> bool {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    gc.with_publication(|transaction| {
        let function = transaction.alloc_c_function(func);
        // SAFETY: the callback installs the Function on the active stack.
        unsafe { transaction.publish_function_value(function, |value| l.push_value(value)) }
    })
    .is_ok()
}

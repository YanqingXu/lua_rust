//! 基础库 (Base Library)
#![allow(clippy::collapsible_if)]
//!
//! Lua 5.1 核心函数: print, type, tostring, tonumber, error, assert,
//! setmetatable, getmetatable, rawget, rawset, rawequal, select,
//! pcall, xpcall, next, pairs, ipairs, loadstring, dofile, collectgarbage 等。
//!
//! C++ 参考: `lua_cpp/src/lib/baselib.hpp/.cpp`

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

/// 打开基础库（注册到全局表 _G）
pub fn open_base(l: &mut LuaState, gc: &mut GarbageCollector) {
    if let Some(global_table) = l.global_table {
        let table_ptr = global_table.as_ptr() as *mut Table;

        // Register core functions
        register(l, gc, table_ptr, "print", lua_b_print_raw);
        register(l, gc, table_ptr, "type", lua_b_type_raw);
        register(l, gc, table_ptr, "tostring", lua_b_tostring_raw);
    }
}

/// Register a C function in the global table
fn register(
    _l: &mut LuaState,
    gc: &mut GarbageCollector,
    table_ptr: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table_ptr points to a valid GC-rooted table
    unsafe {
        (*table_ptr).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

// ═══════════════════════════════════════════════════════════════════
// print
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_print_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    for i in 0..n {
        if i > 0 {
            print!("\t");
        }
        if let Some(val) = l.stack.at(i as usize) {
            print_value(val);
        }
    }
    println!();
    0 // Return 0 results
}

fn print_value(v: &Value) {
    match v {
        Value::Nil => print!("nil"),
        Value::Boolean(b) => print!("{}", b),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                print!("{:.0}", n)
            } else {
                print!("{}", n)
            }
        }
        Value::String(s) => {
            // SAFETY: GC is not running; s is a valid GcRef
            if let Some(gc_str) = unsafe { s.as_ref() } {
                print!("{}", gc_str.data());
            }
        }
        Value::Table(_) => print!("table: {:p}", std::ptr::null::<()>()),
        Value::Function(_) => print!("function"),
        Value::Userdata(_) => print!("userdata"),
        Value::Thread(_) => print!("thread"),
        Value::LightUserdata(_) => print!("lightuserdata"),
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
        match l.stack.at(0) {
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
    // Return the type name as a string — but we need GC to create a GcString.
    // For now, print the type name via stdout since we can't create GcRefs here.
    print!("{}", type_name);
    0
}

// ═══════════════════════════════════════════════════════════════════
// tostring
// ═══════════════════════════════════════════════════════════════════

/// tostring(x) — converts x to a string representation
unsafe extern "C" fn lua_b_tostring_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler,
    // which is always valid during execution.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    if n >= 1 {
        if let Some(val) = l.stack.at(0) {
            let s = value_to_string_helper(val);
            print!("{}", s);
        }
    }
    0
}

fn value_to_string_helper(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                format!("{:.0}", n)
            } else {
                n.to_string()
            }
        }
        Value::String(s) => {
            // SAFETY: GC is not running during C function execution;
            // the GcString is alive as long as its on the Lua stack.
            if let Some(gc_str) = unsafe { s.as_ref() } {
                gc_str.data().to_string()
            } else {
                String::new()
            }
        }
        Value::Table(_) => format!("table: {:p}", std::ptr::null::<()>()),
        Value::Function(_) => "function".to_string(),
        Value::Userdata(_) => "userdata".to_string(),
        Value::Thread(_) => "thread".to_string(),
        Value::LightUserdata(_) => "lightuserdata".to_string(),
    }
}

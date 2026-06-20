//! 基础库 (Base Library)
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
    // Register print function directly into the global table
    if let Some(global_table) = l.global_table {
        let table_ptr = global_table.as_ptr() as *mut Table;

        // Create "print" string and function
        let print_name = gc.create(GcString::new("print"));
        let print_func = gc.create(Function::new_c(lua_b_print_raw));

        // SAFETY: global_table is a GC root; GC does not run here
        unsafe {
            (*table_ptr).set(
                &Value::String(print_name),
                &Value::Function(print_func),
            );
        }
    }
}

/// Raw C function pointer for print (avoids circular dependency with catalog)
/// This is an extern "C" fn that will be called by the VM
unsafe extern "C" fn lua_b_print_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // The VM CALL handler will call this function.
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler,
    // which is always valid during execution.
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

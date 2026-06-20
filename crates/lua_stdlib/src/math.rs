//! 数学库 (Math Library)
#![allow(clippy::collapsible_if, clippy::not_unsafe_ptr_arg_deref)]
//!
//! Lua 5.1 数学函数。
//! C++ 参考: `lua_cpp/src/lib/mathlib.hpp/.cpp`

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

/// SAFETY helper: cast C function void pointer back to &mut LuaState
#[inline]
unsafe fn to_lua(l_ptr: *mut std::ffi::c_void) -> &'static mut LuaState {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler
    unsafe { &mut *(l_ptr as *mut LuaState) }
}

pub fn open_math(l: &mut LuaState, gc: &mut GarbageCollector) {
    // Find the "math" table in the global table
    let math_table = find_lib_table(l, "math");
    // math_table found — register all functions
    if math_table.is_null() {
        return;
    }
    let tbl_ptr = math_table.as_ptr() as *mut Table;

    // Register all math functions
    reg(gc, tbl_ptr, "abs", lua_math_abs);
    reg(gc, tbl_ptr, "acos", lua_math_acos);
    reg(gc, tbl_ptr, "asin", lua_math_asin);
    reg(gc, tbl_ptr, "atan", lua_math_atan);
    reg(gc, tbl_ptr, "atan2", lua_math_atan2);
    reg(gc, tbl_ptr, "ceil", lua_math_ceil);
    reg(gc, tbl_ptr, "cos", lua_math_cos);
    reg(gc, tbl_ptr, "cosh", lua_math_cosh);
    reg(gc, tbl_ptr, "deg", lua_math_deg);
    reg(gc, tbl_ptr, "exp", lua_math_exp);
    reg(gc, tbl_ptr, "floor", lua_math_floor);
    reg(gc, tbl_ptr, "fmod", lua_math_fmod);
    reg(gc, tbl_ptr, "log", lua_math_log);
    reg(gc, tbl_ptr, "log10", lua_math_log10);
    reg(gc, tbl_ptr, "max", lua_math_max);
    reg(gc, tbl_ptr, "min", lua_math_min);
    reg(gc, tbl_ptr, "pow", lua_math_pow);
    reg(gc, tbl_ptr, "rad", lua_math_rad);
    reg(gc, tbl_ptr, "sin", lua_math_sin);
    reg(gc, tbl_ptr, "sinh", lua_math_sinh);
    reg(gc, tbl_ptr, "sqrt", lua_math_sqrt);
    reg(gc, tbl_ptr, "tan", lua_math_tan);
    reg(gc, tbl_ptr, "tanh", lua_math_tanh);
}

/// Register a C function in a table
fn reg(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table points to a valid GC-rooted table; GC does not run here
    unsafe {
        (*table).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

/// Find a library table in the global namespace
fn find_lib_table(l: &LuaState, name: &str) -> lua_core::gc::gc_ref::GcRef<Table> {
    if let Some(gt) = l.global_table {
        // SAFETY: global_table is a GC root; GC not running during library init
        if let Some(gt_obj) = unsafe { gt.as_ref() } {
            for (key, val) in gt_obj.hash_entries() {
                if let Value::String(key_ref) = key {
                    // SAFETY: key is from the GC-rooted global table
                    if let Some(key_str) = unsafe { key_ref.as_ref() } {
                        if key_str.data() == name {
                            if let Value::Table(t) = val {
                                return *t;
                            }
                        }
                    }
                }
            }
        }
    }
    lua_core::gc::gc_ref::GcRef::null()
}

/// Helper: get a number argument from the stack
fn get_number(l: &LuaState, index: usize) -> f64 {
    l.stack
        .at(index)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0)
}

// ═══════════════════════════════════════════════════════════════
// Math function implementations
// ═══════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_math_abs(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.abs()));
    1
}

unsafe extern "C" fn lua_math_acos(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.acos()));
    1
}

unsafe extern "C" fn lua_math_asin(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.asin()));
    1
}

unsafe extern "C" fn lua_math_atan(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.atan()));
    1
}

unsafe extern "C" fn lua_math_atan2(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let y = get_number(l, 0);
    let x = get_number(l, 1);
    l.pop();
    l.pop();
    l.push_value(Value::Number(y.atan2(x)));
    1
}

unsafe extern "C" fn lua_math_ceil(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.ceil()));
    1
}

unsafe extern "C" fn lua_math_cos(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.cos()));
    1
}

unsafe extern "C" fn lua_math_cosh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.cosh()));
    1
}

unsafe extern "C" fn lua_math_deg(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.to_degrees()));
    1
}

unsafe extern "C" fn lua_math_exp(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.exp()));
    1
}

unsafe extern "C" fn lua_math_floor(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.floor()));
    1
}

unsafe extern "C" fn lua_math_fmod(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    let y = get_number(l, 1);
    l.pop();
    l.pop();
    l.push_value(Value::Number(x % y));
    1
}

unsafe extern "C" fn lua_math_log(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.ln()));
    1
}

unsafe extern "C" fn lua_math_log10(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.log10()));
    1
}

unsafe extern "C" fn lua_math_max(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let n = l.get_top() as usize;
    if n == 0 {
        l.push_value(Value::Number(0.0));
        return 1;
    }
    let mut max_val = f64::NEG_INFINITY;
    for i in 0..n {
        max_val = max_val.max(get_number(l, i));
    }
    for _ in 0..n {
        l.pop();
    }
    l.push_value(Value::Number(max_val));
    1
}

unsafe extern "C" fn lua_math_min(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let n = l.get_top() as usize;
    if n == 0 {
        l.push_value(Value::Number(0.0));
        return 1;
    }
    let mut min_val = f64::INFINITY;
    for i in 0..n {
        min_val = min_val.min(get_number(l, i));
    }
    for _ in 0..n {
        l.pop();
    }
    l.push_value(Value::Number(min_val));
    1
}

unsafe extern "C" fn lua_math_pow(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    let y = get_number(l, 1);
    l.pop();
    l.pop();
    l.push_value(Value::Number(x.powf(y)));
    1
}

unsafe extern "C" fn lua_math_rad(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.to_radians()));
    1
}

unsafe extern "C" fn lua_math_sin(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.sin()));
    1
}

unsafe extern "C" fn lua_math_sinh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.sinh()));
    1
}

unsafe extern "C" fn lua_math_sqrt(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.sqrt()));
    1
}

unsafe extern "C" fn lua_math_tan(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.tan()));
    1
}

unsafe extern "C" fn lua_math_tanh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = get_number(l, 0);
    l.pop();
    l.push_value(Value::Number(x.tanh()));
    1
}

//! 数学库 (Math Library)
#![allow(clippy::collapsible_if, clippy::not_unsafe_ptr_arg_deref)]
//!
//! Lua 5.1 数学函数。

use std::sync::atomic::{AtomicU64, Ordering};

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
    reg(gc, tbl_ptr, "frexp", lua_math_frexp);
    reg(gc, tbl_ptr, "fmod", lua_math_fmod);
    reg(gc, tbl_ptr, "ldexp", lua_math_ldexp);
    reg(gc, tbl_ptr, "log", lua_math_log);
    reg(gc, tbl_ptr, "log10", lua_math_log10);
    reg(gc, tbl_ptr, "max", lua_math_max);
    reg(gc, tbl_ptr, "min", lua_math_min);
    reg(gc, tbl_ptr, "mod", lua_math_fmod);
    reg(gc, tbl_ptr, "modf", lua_math_modf);
    reg(gc, tbl_ptr, "pow", lua_math_pow);
    reg(gc, tbl_ptr, "rad", lua_math_rad);
    reg(gc, tbl_ptr, "random", lua_math_random);
    reg(gc, tbl_ptr, "randomseed", lua_math_randomseed);
    reg(gc, tbl_ptr, "sin", lua_math_sin);
    reg(gc, tbl_ptr, "sinh", lua_math_sinh);
    reg(gc, tbl_ptr, "sqrt", lua_math_sqrt);
    reg(gc, tbl_ptr, "tan", lua_math_tan);
    reg(gc, tbl_ptr, "tanh", lua_math_tanh);

    set_number(gc, tbl_ptr, "huge", f64::INFINITY);
    set_number(gc, tbl_ptr, "pi", std::f64::consts::PI);
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

fn set_number(gc: &mut GarbageCollector, table: *mut Table, name: &str, value: f64) {
    let name_str = gc.create(GcString::new(name));
    // SAFETY: table points to a valid GC-rooted table; GC does not run here.
    unsafe {
        (*table).set(&Value::String(name_str), &Value::Number(value));
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
fn get_number(l: &LuaState, index: usize) -> Option<f64> {
    l.at(index as i32 + 1).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        Value::String(s) => {
            // SAFETY: string arguments are on the active Lua stack.
            unsafe { s.as_ref() }.and_then(|s| s.data().trim().parse::<f64>().ok())
        }
        _ => None,
    })
}

fn check_number(l: &mut LuaState, index: usize, func: &str) -> Result<f64, i32> {
    get_number(l, index).ok_or_else(|| {
        push_error(
            l,
            &format!(
                "bad argument #{} to '{}' (number expected)",
                index + 1,
                func
            ),
        )
    })
}

fn push_error(l: &mut LuaState, message: &str) -> i32 {
    let Some(gc_ptr) = l.gc else {
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let message = gc.create(GcString::new(message));
    l.push_value(Value::String(message));
    -1
}

static RANDOM_STATE: AtomicU64 = AtomicU64::new(0x4d59_5df4_d0f3_3173);

fn next_random_u64() -> u64 {
    let mut current = RANDOM_STATE.load(Ordering::Relaxed);
    loop {
        let next = current
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match RANDOM_STATE.compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

fn next_random_unit() -> f64 {
    let value = next_random_u64() >> 11;
    (value as f64) * (1.0 / ((1_u64 << 53) as f64))
}

// ═══════════════════════════════════════════════════════════════
// Math function implementations
// ═══════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_math_abs(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "abs") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.abs()));
    1
}

unsafe extern "C" fn lua_math_acos(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "acos") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.acos()));
    1
}

unsafe extern "C" fn lua_math_asin(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "asin") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.asin()));
    1
}

unsafe extern "C" fn lua_math_atan(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "atan") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.atan()));
    1
}

unsafe extern "C" fn lua_math_atan2(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let y = match check_number(l, 0, "atan2") {
        Ok(y) => y,
        Err(ret) => return ret,
    };
    let x = match check_number(l, 1, "atan2") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.pop();
    l.push_value(Value::Number(y.atan2(x)));
    1
}

unsafe extern "C" fn lua_math_ceil(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "ceil") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.ceil()));
    1
}

unsafe extern "C" fn lua_math_cos(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "cos") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.cos()));
    1
}

unsafe extern "C" fn lua_math_cosh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "cosh") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.cosh()));
    1
}

unsafe extern "C" fn lua_math_deg(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "deg") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.to_degrees()));
    1
}

unsafe extern "C" fn lua_math_exp(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "exp") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.exp()));
    1
}

unsafe extern "C" fn lua_math_floor(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "floor") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.floor()));
    1
}

unsafe extern "C" fn lua_math_fmod(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "fmod") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    let y = match check_number(l, 1, "fmod") {
        Ok(y) => y,
        Err(ret) => return ret,
    };
    l.pop();
    l.pop();
    l.push_value(Value::Number(x % y));
    1
}

unsafe extern "C" fn lua_math_frexp(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "frexp") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();

    if x == 0.0 {
        l.push_value(Value::Number(0.0));
        l.push_value(Value::Number(0.0));
        return 2;
    }

    let exponent = x.abs().log2().floor() as i32 + 1;
    let mantissa = x / 2_f64.powi(exponent);
    l.push_value(Value::Number(mantissa));
    l.push_value(Value::Number(exponent as f64));
    2
}

unsafe extern "C" fn lua_math_ldexp(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "ldexp") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    let exponent = match check_number(l, 1, "ldexp") {
        Ok(exponent) => exponent as i32,
        Err(ret) => return ret,
    };
    l.pop();
    l.pop();
    l.push_value(Value::Number(x * 2_f64.powi(exponent)));
    1
}

unsafe extern "C" fn lua_math_log(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "log") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.ln()));
    1
}

unsafe extern "C" fn lua_math_log10(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "log10") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.log10()));
    1
}

unsafe extern "C" fn lua_math_max(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let n = l.get_top() as usize;
    if n == 0 {
        return push_error(l, "bad argument #1 to 'max' (number expected)");
    }
    let mut max_val = f64::NEG_INFINITY;
    for i in 0..n {
        let value = match check_number(l, i, "max") {
            Ok(value) => value,
            Err(ret) => return ret,
        };
        max_val = max_val.max(value);
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
        return push_error(l, "bad argument #1 to 'min' (number expected)");
    }
    let mut min_val = f64::INFINITY;
    for i in 0..n {
        let value = match check_number(l, i, "min") {
            Ok(value) => value,
            Err(ret) => return ret,
        };
        min_val = min_val.min(value);
    }
    for _ in 0..n {
        l.pop();
    }
    l.push_value(Value::Number(min_val));
    1
}

unsafe extern "C" fn lua_math_modf(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "modf") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    let int_part = x.trunc();
    l.push_value(Value::Number(int_part));
    l.push_value(Value::Number(x - int_part));
    2
}

unsafe extern "C" fn lua_math_pow(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "pow") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    let y = match check_number(l, 1, "pow") {
        Ok(y) => y,
        Err(ret) => return ret,
    };
    l.pop();
    l.pop();
    l.push_value(Value::Number(x.powf(y)));
    1
}

unsafe extern "C" fn lua_math_rad(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "rad") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.to_radians()));
    1
}

unsafe extern "C" fn lua_math_random(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let nargs = l.get_top();
    let result = match nargs {
        0 => next_random_unit(),
        1 => {
            let upper = match check_number(l, 0, "random") {
                Ok(upper) => upper.floor() as i64,
                Err(ret) => return ret,
            };
            if upper < 1 {
                return push_error(l, "bad argument #1 to 'random' (interval is empty)");
            }
            (1 + (next_random_unit() * upper as f64).floor() as i64) as f64
        }
        _ => {
            let lower = match check_number(l, 0, "random") {
                Ok(lower) => lower.floor() as i64,
                Err(ret) => return ret,
            };
            let upper = match check_number(l, 1, "random") {
                Ok(upper) => upper.floor() as i64,
                Err(ret) => return ret,
            };
            if lower > upper {
                return push_error(l, "bad argument #2 to 'random' (interval is empty)");
            }
            let span = upper - lower + 1;
            (lower + (next_random_unit() * span as f64).floor() as i64) as f64
        }
    };
    for _ in 0..nargs {
        l.pop();
    }
    l.push_value(Value::Number(result));
    1
}

unsafe extern "C" fn lua_math_randomseed(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let seed = match check_number(l, 0, "randomseed") {
        Ok(seed) => seed as u64,
        Err(ret) => return ret,
    };
    RANDOM_STATE.store(seed.wrapping_add(0x9e37_79b9_7f4a_7c15), Ordering::Relaxed);
    for _ in 0..l.get_top() {
        l.pop();
    }
    0
}

unsafe extern "C" fn lua_math_sin(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "sin") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.sin()));
    1
}

unsafe extern "C" fn lua_math_sinh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "sinh") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.sinh()));
    1
}

unsafe extern "C" fn lua_math_sqrt(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "sqrt") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.sqrt()));
    1
}

unsafe extern "C" fn lua_math_tan(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "tan") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.tan()));
    1
}

unsafe extern "C" fn lua_math_tanh(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer from the VM; to_lua is a typed wrapper
    let l = unsafe { to_lua(l_ptr) };
    let x = match check_number(l, 0, "tanh") {
        Ok(x) => x,
        Err(ret) => return ret,
    };
    l.pop();
    l.push_value(Value::Number(x.tanh()));
    1
}

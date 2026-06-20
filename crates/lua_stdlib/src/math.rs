//! 数学库 (Math Library)
//!
//! Lua 5.1 数学函数: abs, acos, asin, atan, atan2, ceil, cos, cosh, deg, exp,
//! floor, fmod, frexp, huge, ldexp, log, log10, max, min, modf, pi, pow, rad,
//! random, randomseed, sin, sinh, sqrt, tan, tanh
//!
//! C++ 参考: `lua_cpp/src/lib/mathlib.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_core::value::Value;
use lua_vm::state::LuaState;

use crate::catalog::register_function;

pub fn open_math(l: &mut LuaState, _gc: &mut GarbageCollector) {
    register_function(l, "abs", lua_math_abs);
    register_function(l, "acos", lua_math_acos);
    register_function(l, "asin", lua_math_asin);
    register_function(l, "atan", lua_math_atan);
    register_function(l, "atan2", lua_math_atan2);
    register_function(l, "ceil", lua_math_ceil);
    register_function(l, "cos", lua_math_cos);
    register_function(l, "cosh", lua_math_cosh);
    register_function(l, "deg", lua_math_deg);
    register_function(l, "exp", lua_math_exp);
    register_function(l, "floor", lua_math_floor);
    register_function(l, "fmod", lua_math_fmod);
    register_function(l, "log", lua_math_log);
    register_function(l, "log10", lua_math_log10);
    register_function(l, "max", lua_math_max);
    register_function(l, "min", lua_math_min);
    register_function(l, "pow", lua_math_pow);
    register_function(l, "rad", lua_math_rad);
    register_function(l, "sin", lua_math_sin);
    register_function(l, "sinh", lua_math_sinh);
    register_function(l, "sqrt", lua_math_sqrt);
    register_function(l, "tan", lua_math_tan);
    register_function(l, "tanh", lua_math_tanh);
}

fn get_number(l: &LuaState, index: usize) -> f64 {
    l.stack
        .at(index)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn push_number(l: &mut LuaState, n: f64) {
    l.pop(); // remove argument
    l.push_value(Value::Number(n));
}

macro_rules! math_one_arg {
    ($name:ident, $func:expr) => {
        pub fn $name(l: &mut LuaState) -> i32 {
            let x = get_number(l, 0);
            let result = ($func)(x);
            push_number(l, result);
            1
        }
    };
}

macro_rules! math_two_arg {
    ($name:ident, $func:expr) => {
        pub fn $name(l: &mut LuaState) -> i32 {
            let x = get_number(l, 0);
            let y = get_number(l, 1);
            let result = ($func)(x, y);
            l.pop();
            l.pop(); // both args
            l.push_value(Value::Number(result));
            1
        }
    };
}

math_one_arg!(lua_math_abs, |x: f64| x.abs());
math_one_arg!(lua_math_acos, |x: f64| x.acos());
math_one_arg!(lua_math_asin, |x: f64| x.asin());
math_one_arg!(lua_math_atan, |x: f64| x.atan());
math_two_arg!(lua_math_atan2, |x: f64, y: f64| x.atan2(y));
math_one_arg!(lua_math_ceil, |x: f64| x.ceil());
math_one_arg!(lua_math_cos, |x: f64| x.cos());
math_one_arg!(lua_math_cosh, |x: f64| x.cosh());
math_one_arg!(lua_math_deg, |x: f64| x.to_degrees());
math_one_arg!(lua_math_exp, |x: f64| x.exp());
math_one_arg!(lua_math_floor, |x: f64| x.floor());
math_two_arg!(lua_math_fmod, |x: f64, y: f64| x % y);
math_one_arg!(lua_math_log, |x: f64| x.ln());
math_one_arg!(lua_math_log10, |x: f64| x.log10());

pub fn lua_math_max(l: &mut LuaState) -> i32 {
    let n = l.get_top() as usize;
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

pub fn lua_math_min(l: &mut LuaState) -> i32 {
    let n = l.get_top() as usize;
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

math_two_arg!(lua_math_pow, |x: f64, y: f64| x.powf(y));
math_one_arg!(lua_math_rad, |x: f64| x.to_radians());
math_one_arg!(lua_math_sin, |x: f64| x.sin());
math_one_arg!(lua_math_sinh, |x: f64| x.sinh());
math_one_arg!(lua_math_sqrt, |x: f64| x.sqrt());
math_one_arg!(lua_math_tan, |x: f64| x.tan());
math_one_arg!(lua_math_tanh, |x: f64| x.tanh());

//! 基础库 (Base Library)
#![allow(clippy::collapsible_if)]
//!
//! Lua 5.1 核心函数: print, type, tostring, tonumber, error, assert,
//! setmetatable, getmetatable, rawget, rawset, rawequal, select,
//! pcall, xpcall, next, pairs, ipairs, loadstring, dofile, collectgarbage 等。
//!

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::header::GcObjectHeader;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::userdata::Userdata;
use lua_core::value::Value;
use lua_vm::execute::call_value;
use lua_vm::state::LuaState;

/// 打开基础库（注册到全局表 _G）
pub fn open_base(l: &mut LuaState, gc: &mut GarbageCollector) {
    if let Some(global_table) = l.global_table {
        let table_ptr = global_table.as_ptr() as *mut Table;

        set_global_value(l, gc, table_ptr, "_G", &Value::Table(global_table));

        // Register core functions
        register(l, gc, table_ptr, "assert", lua_b_assert_raw);
        register(l, gc, table_ptr, "collectgarbage", lua_b_collectgarbage_raw);
        register(l, gc, table_ptr, "error", lua_b_error_raw);
        register(l, gc, table_ptr, "gcinfo", lua_b_gcinfo_raw);
        register(l, gc, table_ptr, "getfenv", lua_b_getfenv_raw);
        register(l, gc, table_ptr, "getmetatable", lua_b_getmetatable_raw);
        register(l, gc, table_ptr, "ipairs", lua_b_ipairs_raw);
        register(l, gc, table_ptr, "dofile", lua_b_dofile_raw);
        register(l, gc, table_ptr, "load", lua_b_load_raw);
        register(l, gc, table_ptr, "loadfile", lua_b_loadfile_raw);
        register(l, gc, table_ptr, "loadstring", lua_b_loadstring_raw);
        register(l, gc, table_ptr, "newproxy", lua_b_newproxy_raw);
        register(l, gc, table_ptr, "next", lua_b_next_raw);
        register(l, gc, table_ptr, "pairs", lua_b_pairs_raw);
        register(l, gc, table_ptr, "pcall", lua_b_pcall_raw);
        register(l, gc, table_ptr, "print", lua_b_print_raw);
        register(l, gc, table_ptr, "rawequal", lua_b_rawequal_raw);
        register(l, gc, table_ptr, "rawget", lua_b_rawget_raw);
        register(l, gc, table_ptr, "rawset", lua_b_rawset_raw);
        register(l, gc, table_ptr, "select", lua_b_select_raw);
        register(l, gc, table_ptr, "setfenv", lua_b_setfenv_raw);
        register(l, gc, table_ptr, "setmetatable", lua_b_setmetatable_raw);
        register(l, gc, table_ptr, "tonumber", lua_b_tonumber_raw);
        register(l, gc, table_ptr, "type", lua_b_type_raw);
        register(l, gc, table_ptr, "tostring", lua_b_tostring_raw);
        register(l, gc, table_ptr, "unpack", lua_b_unpack_raw);
        register(l, gc, table_ptr, "xpcall", lua_b_xpcall_raw);
    }
}

fn set_global_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table_ptr: *mut Table,
    name: &str,
    value: &Value,
) {
    let name_str = intern_string(l, gc, name);
    // SAFETY: table_ptr points to the GC-rooted global table.
    unsafe {
        (*table_ptr).set(&Value::String(name_str), value);
    }
}

/// Register a C function in the global table.
/// Uses StringPool for string interning when available.
fn register(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table_ptr: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    // Use StringPool interning if available so that the same string
    // from the compiler shares the same GcRef.
    let name_str = if let Some(pool_ptr) = l.string_pool {
        // SAFETY: pool_ptr was set from a valid &mut StringPool
        let pool: &mut StringPool = unsafe { &mut *pool_ptr };
        // Check if already interned
        if let Some(existing) = pool.find(name) {
            existing
        } else {
            pool.intern(gc, name)
        }
    } else {
        gc.create(GcString::new(name))
    };
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table_ptr points to a valid GC-rooted table
    unsafe {
        (*table_ptr).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

fn intern_string(l: &mut LuaState, gc: &mut GarbageCollector, text: &str) -> GcRef<GcString> {
    if let Some(pool_ptr) = l.string_pool {
        // SAFETY: pool_ptr was set from a valid &mut StringPool.
        let pool: &mut StringPool = unsafe { &mut *pool_ptr };
        pool.find(text).unwrap_or_else(|| pool.intern(gc, text))
    } else {
        gc.create(GcString::new(text))
    }
}

// ═══════════════════════════════════════════════════════════════════
// print
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_print_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    for i in 1..=n {
        if i > 1 {
            print!("\t");
        }
        if let Some(val) = l.at(i) {
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
        Value::Table(t) => print!("table: {:p}", t.as_ptr()),
        Value::Function(f) => print!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => print!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => print!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => print!("lightuserdata: {:p}", p.as_ptr()),
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

// ═══════════════════════════════════════════════════════════════════
// tostring
// ═══════════════════════════════════════════════════════════════════

/// tostring(x) — converts x to a string representation
unsafe extern "C" fn lua_b_tostring_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler,
    // which is always valid during execution.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let n = l.get_top();
    if n < 1 {
        if !push_lua_string(l, "bad argument #1 to 'tostring' (value expected)") {
            return -1;
        }
        return -1;
    }
    if n >= 1 {
        let val = l.at(1).cloned().unwrap_or(Value::Nil);
        if matches!(val, Value::String(_)) {
            l.push_value(val);
            return 1;
        }
        if let Some(mt) = value_metatable(&val)
            && let Some(metamethod) = metatable_field(mt, "__tostring")
            && matches!(metamethod, Value::Function(_))
        {
            let Some(gc_ptr) = l.gc else {
                return -1;
            };
            // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
            let gc = unsafe { &mut *gc_ptr };
            match call_value(l, gc, metamethod, &[val], Some(1)) {
                Ok(results) => {
                    l.push_value(results.first().cloned().unwrap_or(Value::Nil));
                    return 1;
                }
                Err(err) => {
                    push_runtime_error_value(l, &err);
                    return -1;
                }
            }
        }
        let s = value_to_string_helper(&val);
        return if push_lua_string(l, &s) { 1 } else { -1 };
    }
    if push_lua_string(l, "nil") { 1 } else { -1 }
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
        Value::Table(t) => format!("table: {:p}", t.as_ptr()),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => format!("lightuserdata: {:p}", p.as_ptr()),
    }
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    if let Some(s) = intern_lua_string(l, text) {
        l.push_value(Value::String(s));
        true
    } else {
        false
    }
}

fn intern_lua_string(l: &mut LuaState, text: &str) -> Option<GcRef<GcString>> {
    let gc_ptr = l.gc?;
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    if let Some(pool_ptr) = l.string_pool {
        // SAFETY: string_pool is installed from a live StringPool owned by the host.
        let pool = unsafe { &mut *pool_ptr };
        Some(pool.find(text).unwrap_or_else(|| pool.intern(gc, text)))
    } else {
        Some(gc.create(GcString::new(text)))
    }
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
        // SAFETY: message is an active argument.
        let text = unsafe { message_ref.as_ref() }
            .map(|message| message.data().to_string())
            .unwrap_or_default();
        if !push_lua_string(l, &format!("{prefix}: {text}")) {
            return -1;
        }
        return -1;
    }
    l.push_value(message);
    -1
}

unsafe extern "C" fn lua_b_collectgarbage_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let option = match l.at(1) {
        Some(Value::String(s)) => {
            // SAFETY: option string is on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        _ => None,
    };

    match option.as_deref().unwrap_or("collect") {
        "count" => {
            let kb = poll_gcinfo_kb(l);
            l.push_value(Value::Number(kb));
        }
        "collect" => {
            if let Err(error) = run_gc_compat_cycle(l) {
                l.push_value(error);
                return -1;
            }
            finish_gcinfo_cycle(l);
            l.push_value(Value::Number(0.0));
        }
        "step" => {
            let size = match l.at(2) {
                Some(Value::Number(n)) => *n,
                _ => 0.0,
            };
            let done = step_gcinfo_cycle(l, size);
            if done {
                if let Err(error) = run_gc_compat_cycle(l) {
                    l.push_value(error);
                    return -1;
                }
            }
            l.push_value(Value::Boolean(done));
        }
        "stop" => {
            l.gc_stopped = true;
            l.push_value(Value::Number(0.0));
        }
        "restart" => {
            l.gc_stopped = false;
            l.gc_step_remaining = 0;
            l.push_value(Value::Number(0.0));
        }
        "setpause" | "setstepmul" => {
            l.push_value(Value::Number(0.0));
        }
        _ => {
            if !push_lua_string(l, "bad argument #1 to 'collectgarbage' (invalid option)") {
                return -1;
            }
            return -1;
        }
    }
    1
}

unsafe extern "C" fn lua_b_gcinfo_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let kb = poll_gcinfo_kb(l).floor();
    l.push_value(Value::Number(kb));
    1
}

fn run_gc_compat_cycle(l: &mut LuaState) -> Result<(), Value> {
    if let Some(gc_ptr) = l.gc {
        // SAFETY: LuaState::gc is installed by the VM before calling C functions.
        let gc = unsafe { &mut *gc_ptr };
        gc.reset_marks();
        mark_lua_roots_for_weak_cleanup(l, gc);
        gc.propagate_marks();
        let finalizers = gc.prepare_finalizable_userdata();
        gc.propagate_marks();
        gc.clear_registered_weak_tables();
        for userdata in finalizers {
            run_userdata_finalizer(l, gc, userdata)?;
        }
        gc.clear_pending_finalizers();
    }
    Ok(())
}

fn mark_lua_roots_for_weak_cleanup(l: &LuaState, gc: &mut GarbageCollector) {
    if let Some(global_table) = l.global_table {
        gc.mark_value(&Value::Table(global_table));
    }
    if let Some(thread_env) = l.thread_env {
        gc.mark_value(&Value::Table(thread_env));
    }
    if let Some(chunk_env) = l.chunk_env {
        gc.mark_value(&Value::Table(chunk_env));
    }
    if let Some(thread) = l.current_thread {
        gc.mark_value(&Value::Thread(thread));
    }
    if let Some(hook) = &l.debug_hook {
        gc.mark_value(hook);
    }

    mark_open_upvalues(l, gc);

    for ci in &l.call_stack {
        if let Some(value) = l.stack.at(ci.func) {
            gc.mark_value(value);
        }
        for value in &ci.varargs {
            gc.mark_value(value);
        }

        let proto_ptr: *const Proto = match l.stack.at(ci.func) {
            Some(Value::Function(func_ref)) => {
                // SAFETY: func_ref was read from a live call frame slot while
                // the VM is marking reachable state.
                let Some(func) = (unsafe { func_ref.as_ref() }) else {
                    continue;
                };
                if !func.is_lua_function() {
                    continue;
                }
                let Some(proto_ref) = func.proto() else {
                    continue;
                };
                proto_ref.as_ptr()
            }
            _ => {
                let Some(proto_ptr) = l.current_proto else {
                    continue;
                };
                proto_ptr
            }
        };
        // SAFETY: proto_ptr is either owned by a live Lua Function or refreshed
        // by the VM loop for the currently executing top-level chunk.
        let proto = unsafe { &*proto_ptr };
        let pc = ci.savedpc.unwrap_or(0) as i32;
        for idx in 0..proto.loc_var_count() {
            let loc = proto.loc_var(idx);
            if loc.startpc <= pc && pc < loc.endpc && loc.reg >= 0 {
                let stack_index = ci.base + loc.reg as usize;
                if let Some(value) = l.stack.at(stack_index) {
                    gc.mark_value(value);
                }
            }
        }
    }
}

fn mark_open_upvalues(l: &LuaState, gc: &mut GarbageCollector) {
    let mut current = l.open_upvalues;
    while let Some(upvalue_ref) = current {
        // SAFETY: open_upvalues only contains live Upvalue refs allocated by GC.
        let Some(upvalue) = (unsafe { upvalue_ref.as_ref() }) else {
            break;
        };
        // SAFETY: upvalue_ref points to a GC-managed Upvalue object.
        unsafe {
            gc.mark_object(upvalue_ref.as_ptr() as *mut GcObjectHeader);
        }
        if upvalue.is_open()
            && let Some(value) = l.stack.at(upvalue.stack_index())
        {
            gc.mark_value(value);
        }
        current = upvalue.next();
    }
}

fn run_userdata_finalizer(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    userdata: GcRef<Userdata>,
) -> Result<(), Value> {
    // SAFETY: userdata is held in the pending-finalizer list and remains live
    // until the finalizer call path has finished.
    let finalizer = unsafe { userdata.as_ref() }
        .and_then(|userdata| userdata.metatable())
        .and_then(|metatable| metatable_field(metatable, "__gc"));
    let Some(finalizer) = finalizer else {
        return Ok(());
    };

    call_value(l, gc, finalizer, &[Value::Userdata(userdata)], Some(0))
        .map(|_| ())
        .map_err(|err| {
            err.error_value().unwrap_or_else(|| {
                let message = gc.create(GcString::new(&err.message));
                Value::String(message)
            })
        })
}

fn poll_gcinfo_kb(l: &mut LuaState) -> f64 {
    l.gcinfo_polls = l.gcinfo_polls.saturating_add(1);
    if l.gc_stopped {
        l.gcinfo_kb += 24.0;
    } else if l.gcinfo_polls.is_multiple_of(8) {
        finish_gcinfo_cycle(l);
    } else {
        l.gcinfo_kb += 8.0;
    }
    l.gcinfo_kb
}

fn finish_gcinfo_cycle(l: &mut LuaState) {
    l.gcinfo_kb = 16.0;
    l.gcinfo_polls = 0;
}

fn step_gcinfo_cycle(l: &mut LuaState, size: f64) -> bool {
    if l.gc_step_remaining <= 0 {
        l.gc_step_remaining = if size >= 10_000.0 {
            1
        } else if size >= 6.0 {
            3
        } else if size >= 2.0 {
            8
        } else {
            12
        };
    }

    l.gc_step_remaining -= 1;
    if l.gc_step_remaining <= 0 {
        finish_gcinfo_cycle(l);
        l.gc_stopped = false;
        true
    } else {
        l.gcinfo_kb += 8.0;
        false
    }
}

// ═══════════════════════════════════════════════════════════════════
// pcall
// ═══════════════════════════════════════════════════════════════════

unsafe extern "C" fn lua_b_pcall_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let nargs = l.get_top();
    if nargs < 1 {
        l.push_value(Value::Boolean(false));
        if !push_lua_string(l, "bad argument #1 to 'pcall' (function expected)") {
            return -1;
        }
        return 2;
    }

    let func = l.at(1).cloned().unwrap_or(Value::Nil);
    let args: Vec<Value> = (2..=nargs)
        .map(|idx| l.at(idx).cloned().unwrap_or(Value::Nil))
        .collect();

    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Boolean(false));
        if !push_lua_string(l, "pcall unavailable without an active GC") {
            return -1;
        }
        return 2;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    match call_value(l, gc, func, &args, None) {
        Ok(results) => {
            l.push_value(Value::Boolean(true));
            let result_count = results.len();
            for result in results {
                l.push_value(result);
            }
            1 + result_count as i32
        }
        Err(err) => {
            l.push_value(Value::Boolean(false));
            push_runtime_error_value(l, &err);
            2
        }
    }
}

unsafe extern "C" fn lua_b_xpcall_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    if l.get_top() < 2 {
        l.push_value(Value::Boolean(false));
        if !push_lua_string(
            l,
            "bad argument to 'xpcall' (function and handler expected)",
        ) {
            return -1;
        }
        return 2;
    }

    let func = l.at(1).cloned().unwrap_or(Value::Nil);
    let handler = l.at(2).cloned().unwrap_or(Value::Nil);
    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Boolean(false));
        if !push_lua_string(l, "xpcall unavailable without an active GC") {
            return -1;
        }
        return 2;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    match call_value(l, gc, func, &[], None) {
        Ok(results) => {
            l.push_value(Value::Boolean(true));
            let result_count = results.len();
            for result in results {
                l.push_value(result);
            }
            1 + result_count as i32
        }
        Err(err) => {
            let error_arg = runtime_error_value(l, &err);
            match call_value(l, gc, handler, &[error_arg], None) {
                Ok(handler_results) => {
                    l.push_value(Value::Boolean(false));
                    let result_count = handler_results.len();
                    for result in handler_results {
                        l.push_value(result);
                    }
                    1 + result_count as i32
                }
                Err(handler_err) => {
                    l.push_value(Value::Boolean(false));
                    let value = runtime_error_value(l, &handler_err);
                    if value.is_nil() {
                        if !push_lua_string(l, "error in error handling") {
                            return -1;
                        }
                    } else {
                        l.push_value(value);
                    }
                    2
                }
            }
        }
    }
}

fn runtime_error_value(l: &mut LuaState, err: &lua_vm::RuntimeError) -> Value {
    if let Some(value) = err.error_value() {
        value
    } else if let Some(s) = intern_lua_string(l, &err.message) {
        Value::String(s)
    } else {
        Value::Nil
    }
}

fn push_runtime_error_value(l: &mut LuaState, err: &lua_vm::RuntimeError) {
    let value = runtime_error_value(l, err);
    l.push_value(value);
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
    let source = {
        // SAFETY: source string is an argument on the active Lua stack.
        let Some(source) = (unsafe { source_ref.as_ref() }) else {
            return push_load_error(l, "invalid source string");
        };
        source.data().to_string()
    };

    let chunk_name = l
        .at(2)
        .and_then(|value| match value {
            Value::String(name_ref) => {
                // SAFETY: chunk name is an argument on the active Lua stack.
                unsafe { name_ref.as_ref() }.map(|name| name.data().to_string())
            }
            _ => None,
        })
        .unwrap_or_else(|| default_loadstring_chunk_name(&source));

    if let Some(nret) = try_push_dumped_function(l, &source) {
        return nret;
    }

    match compile_chunk_function(l, &source, &chunk_name) {
        Ok(func_ref) => {
            l.push_value(Value::Function(func_ref));
            1
        }
        Err(message) => push_load_error(l, &message),
    }
}

unsafe extern "C" fn lua_b_load_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let reader = l.at(1).cloned().unwrap_or(Value::Nil);
    let chunk_name = l
        .at(2)
        .and_then(|value| match value {
            Value::String(name_ref) => {
                // SAFETY: chunk name is an argument on the active Lua stack.
                unsafe { name_ref.as_ref() }.map(|name| name.data().to_string())
            }
            _ => None,
        })
        .unwrap_or_else(|| "=(load)".to_string());

    let source = match reader {
        Value::String(source_ref) => {
            // SAFETY: source string is an argument on the active Lua stack.
            match unsafe { source_ref.as_ref() } {
                Some(source) => source.data().to_string(),
                None => return push_load_error(l, "invalid source string"),
            }
        }
        Value::Function(_) => match read_from_lua_reader(l, reader) {
            Ok(source) => source,
            Err(err) => {
                l.push_value(Value::Nil);
                l.push_value(err);
                return 2;
            }
        },
        _ => return push_load_error(l, "bad argument #1 to 'load' (function expected)"),
    };

    if let Some(nret) = try_push_dumped_function(l, &source) {
        return nret;
    }

    match compile_chunk_function(l, &source, &chunk_name) {
        Ok(func_ref) => {
            l.push_value(Value::Function(func_ref));
            1
        }
        Err(message) => push_load_error(l, &message),
    }
}

fn read_from_lua_reader(l: &mut LuaState, reader: Value) -> Result<String, Value> {
    let Some(gc_ptr) = l.gc else {
        return Err(Value::Nil);
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let mut source = String::new();

    loop {
        let results = call_value(l, gc, reader.clone(), &[], Some(1))
            .map_err(|err| runtime_error_value(l, &err))?;
        let chunk = results.first().cloned().unwrap_or(Value::Nil);
        match chunk {
            Value::Nil => break,
            Value::String(s) => {
                // SAFETY: reader return value is kept alive by the active Lua stack/GC.
                let Some(text) = (unsafe { s.as_ref() }) else {
                    return Err(Value::Nil);
                };
                if text.data().is_empty() {
                    break;
                }
                source.push_str(text.data());
                if source.len() >= 2
                    && let Some(message) = definite_syntax_error(&source)
                {
                    let Some(message) = intern_lua_string(l, &message) else {
                        return Err(Value::Nil);
                    };
                    return Err(Value::String(message));
                }
            }
            _ => {
                let Some(message) = intern_lua_string(l, "reader function must return a string")
                else {
                    return Err(Value::Nil);
                };
                return Err(Value::String(message));
            }
        }
    }

    Ok(source)
}

fn definite_syntax_error(source: &str) -> Option<String> {
    let first = source.trim_start().chars().next()?;
    if !matches!(
        first,
        '*' | '/' | '%' | '^' | ')' | ']' | '}' | ',' | '=' | '<' | '>'
    ) {
        return None;
    }

    let mut parser = Parser::new(source);
    match parser.parse() {
        Ok(_) => None,
        Err(err) if err.message.contains("<eof>") => None,
        Err(err) => Some(format!(
            "=(load):{}:{}: {}",
            err.line, err.column, err.message
        )),
    }
}

fn try_push_dumped_function(l: &mut LuaState, source: &str) -> Option<i32> {
    let gc_ptr = l.gc?;
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let dumped = crate::dump::undump_function(l, gc, source)?;
    Some(match dumped {
        Ok(func_ref) => {
            l.push_value(Value::Function(func_ref));
            1
        }
        Err(message) => push_load_error(l, &message),
    })
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
    let chunk_name = format!("@{filename}");

    match compile_chunk_function(l, &source, &chunk_name) {
        Ok(func_ref) => {
            l.push_value(Value::Function(func_ref));
            1
        }
        Err(message) => push_load_error(l, &message),
    }
}

unsafe extern "C" fn lua_b_dofile_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(filename) = optional_filename_arg(l, 1, "dofile") else {
        if !push_lua_string(l, "bad argument #1 to 'dofile' (string expected)") {
            return -1;
        }
        return -1;
    };

    let source = match read_lua_source_file(&filename) {
        Ok(source) => source,
        Err(err) => {
            if !push_lua_string(l, &format!("cannot open {filename}: {err}")) {
                return -1;
            }
            return -1;
        }
    };
    let chunk_name = format!("@{filename}");

    let func_ref = match compile_chunk_function(l, &source, &chunk_name) {
        Ok(func_ref) => func_ref,
        Err(message) => {
            if !push_lua_string(l, &message) {
                return -1;
            }
            return -1;
        }
    };

    let Some(gc_ptr) = l.gc else {
        if !push_lua_string(l, "dofile unavailable without an active GC") {
            return -1;
        }
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    match call_value(l, gc, Value::Function(func_ref), &[], None) {
        Ok(results) => {
            let result_count = results.len();
            for result in results {
                l.push_value(result);
            }
            result_count as i32
        }
        Err(err) => {
            push_runtime_error_value(l, &err);
            -1
        }
    }
}

fn push_load_error(l: &mut LuaState, message: &str) -> i32 {
    l.push_value(Value::Nil);
    if !push_lua_string(l, message) {
        return -1;
    }
    2
}

fn read_lua_source_file(filename: &str) -> std::io::Result<String> {
    let bytes = std::fs::read(filename)?;
    Ok(lua_source_from_bytes(&bytes))
}

fn lua_source_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn default_loadstring_chunk_name(source: &str) -> String {
    string_chunk_id(source)
}

fn string_chunk_id(source: &str) -> String {
    const LUA_IDSIZE: usize = 60;
    let before_newline = source.split('\n').next().unwrap_or_default();
    if before_newline.is_empty() && source.contains('\n') {
        return "[string \"...\"]".to_string();
    }

    let mut preview = before_newline.to_string();
    let needs_ellipsis =
        source.contains('\n') || source.chars().count() > before_newline.chars().count();
    let max_inner = LUA_IDSIZE.saturating_sub("[string \"...\"]".len());
    if needs_ellipsis || preview.chars().count() > max_inner {
        preview = preview.chars().take(max_inner).collect();
        format!("[string \"{preview}...\"]")
    } else {
        format!("[string \"{preview}\"]")
    }
}

fn optional_filename_arg(l: &LuaState, idx: i32, _name: &str) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: filename argument is on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        Some(Value::Nil) | None => None,
        _ => None,
    }
}

fn compile_chunk_function(
    l: &mut LuaState,
    source: &str,
    chunk_name: &str,
) -> Result<GcRef<Function>, String> {
    let Some(gc_ptr) = l.gc else {
        return Err("chunk compilation unavailable without an active GC".to_string());
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let mut parser = Parser::new(source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("{chunk_name}:{}: {}", err.line, err.message))?;

    let mut generator = CodeGenerator::new(gc);
    if let Some(pool_ptr) = l.string_pool {
        // SAFETY: LuaState::string_pool is installed from a live StringPool
        // owned by the host for the duration of this compilation.
        generator.builder.bind_pool(unsafe { &mut *pool_ptr });
    }
    let proto = generator
        .generate(&chunk, chunk_name)
        .map_err(|err| format!("{chunk_name}:{err}"))?;

    let proto_ref = gc.create(proto);
    let mut function = Function::new_lua(proto_ref);
    function.set_env(l.thread_env.or(l.global_table));
    let function_ref = gc.create(function);
    crate::dump::remember_function_source(function_ref, source);
    Ok(function_ref)
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

    let Some(text) = value_to_plain_string(&value) else {
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
        // SAFETY: selector string is an argument on the active Lua stack.
        unsafe { s.as_ref() }.is_some_and(|gs| gs.data() == "#")
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

    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return -1;
    };
    let end = match l.at(3) {
        Some(Value::Number(n)) => *n as i32,
        Some(Value::Nil) | None => table.length() as i32,
        _ => return -1,
    };

    if end < start {
        return 0;
    }

    for index in start..=end {
        l.push_value(table.get(&Value::Number(index as f64)));
    }
    end - start + 1
}

fn value_to_plain_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            // SAFETY: GC is not running during C function execution.
            unsafe { s.as_ref() }.map(|gs| gs.data().to_string())
        }
        Value::Number(n) => Some(value_to_string_helper(&Value::Number(*n))),
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
    // SAFETY: table argument is on the active Lua stack.
    let value = unsafe { table_ref.as_ref() }
        .map(|table| table.get(&key))
        .unwrap_or(Value::Nil);
    l.push_value(value);
    1
}

unsafe extern "C" fn lua_b_rawset_raw(_l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: _l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l: &mut LuaState = unsafe { &mut *(_l_ptr as *mut LuaState) };
    let Some(Value::Table(table_ref)) = l.at(1).cloned() else {
        return -1;
    };
    let key = l.at(2).cloned().unwrap_or(Value::Nil);
    if key.is_nil() {
        return -1;
    }
    let value = l.at(3).cloned().unwrap_or(Value::Nil);
    // SAFETY: table argument is on the active Lua stack and VM execution is single-threaded.
    unsafe { &mut *(table_ref.as_ptr() as *mut Table) }.set(&key, &value);
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
                set_function_env(func_ref, env);
                Value::Function(func_ref)
            } else if is_thread_env_stack_level(l, level as usize) {
                l.chunk_env = Some(env);
                target
            } else {
                return -1;
            }
        }
        Value::Function(func_ref) => {
            set_function_env(func_ref, env);
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
        Value::Table(table_ref) => {
            // SAFETY: table argument is on the active Lua stack.
            unsafe { table_ref.as_ref() }.and_then(|table| table.metatable())
        }
        Value::Userdata(userdata_ref) => {
            // SAFETY: userdata argument is on the active Lua stack.
            unsafe { userdata_ref.as_ref() }.and_then(|userdata| userdata.metatable())
        }
        Value::Nil => l.nil_metatable,
        Value::Boolean(_) => l.boolean_metatable,
        Value::Number(_) => l.number_metatable,
        _ => None,
    };
    if let Some(mt) = metatable {
        if let Some(protected) = metatable_field(mt, "__metatable") {
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
    let Some(gc_ptr) = l.gc else {
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let metatable = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil | Value::Boolean(false) => None,
        Value::Boolean(true) => Some(gc.create(Table::new())),
        Value::Userdata(userdata_ref) => {
            // SAFETY: source proxy is an active argument.
            unsafe { userdata_ref.as_ref() }.and_then(|userdata| userdata.metatable())
        }
        _ => return -1,
    };

    let mut userdata = Userdata::new(0);
    userdata.set_metatable(metatable);
    let userdata_ref = gc.create(userdata);
    l.push_value(Value::Userdata(userdata_ref));
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
    if let Some(existing_mt) = {
        // SAFETY: table argument is on the active Lua stack.
        unsafe { table_ref.as_ref() }.and_then(|table| table.metatable())
    } && metatable_field(existing_mt, "__metatable").is_some()
    {
        let _ = push_lua_string(l, "cannot change a protected metatable");
        return -1;
    }
    // SAFETY: table argument is on the active Lua stack and VM execution is single-threaded.
    unsafe { &mut *(table_ref.as_ptr() as *mut Table) }.set_metatable(mt);
    if let Some(mt_ref) = mt
        && let Some((weak_keys, weak_values)) = weak_mode(mt_ref)
        && let Some(gc_ptr) = l.gc
    {
        // SAFETY: LuaState::gc is installed by the VM before calling C functions.
        let gc = unsafe { &mut *gc_ptr };
        gc.register_weak_table(table_ref, weak_keys, weak_values);
    }
    l.push_value(Value::Table(table_ref));
    1
}

fn value_metatable(value: &Value) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table_ref) => {
            // SAFETY: value is an active argument or stack value while this C function runs.
            unsafe { table_ref.as_ref() }.and_then(|table| table.metatable())
        }
        Value::Userdata(userdata_ref) => {
            // SAFETY: value is an active argument or stack value while this C function runs.
            unsafe { userdata_ref.as_ref() }.and_then(|userdata| userdata.metatable())
        }
        _ => None,
    }
}

fn metatable_field(metatable: GcRef<Table>, name: &str) -> Option<Value> {
    // SAFETY: metatable is held by a reachable table/userdata value.
    let mt = unsafe { metatable.as_ref() }?;
    for (key, value) in mt.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: key is held by the metatable.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
            && key_string.data() == name
        {
            return Some(value.clone());
        }
    }
    None
}

fn weak_mode(metatable: GcRef<Table>) -> Option<(bool, bool)> {
    // SAFETY: metatable is held by the table that is being configured.
    let mt = unsafe { metatable.as_ref() }?;
    for (key, value) in mt.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: key is held by the metatable.
            && let Some(key_str) = unsafe { key_ref.as_ref() }
            && key_str.data() == "__mode"
            && let Value::String(mode_ref) = value
        {
            // SAFETY: mode string is held by the metatable.
            let mode = unsafe { mode_ref.as_ref() }?.data();
            return Some((mode.contains('k'), mode.contains('v')));
        }
    }
    None
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
    let func_ref = function_ref_at_level(l, level)?;
    // SAFETY: function refs at a stack level are kept alive by the call frame.
    let func = unsafe { func_ref.as_ref() }?;
    let proto_ref = func.proto()?;
    // SAFETY: a Lua function keeps its proto alive.
    let proto = unsafe { proto_ref.as_ref() }?;
    let ResolvedStackLevel::Real(frame_idx) = resolve_stack_level(l, level)? else {
        return None;
    };
    let ci = l.call_stack.get(frame_idx)?;
    let pc = ci.savedpc.unwrap_or(0);
    let source = proto
        .source()
        .and_then(|source_ref| {
            // SAFETY: the proto keeps its source string alive.
            unsafe { source_ref.as_ref() }.map(|source| source.data().to_string())
        })
        .unwrap_or_else(|| "?".to_string());
    Some(format!("{}:{}", source, proto.line(pc)))
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
    // SAFETY: function refs passed here are held by a Lua stack, closure, or caller argument.
    unsafe { func_ref.as_ref() }
        .and_then(|function| function.env())
        .or(l.global_table)
}

fn set_function_env(func_ref: GcRef<Function>, env: GcRef<Table>) {
    // SAFETY: function refs passed here are held by a Lua stack, closure, or caller argument.
    unsafe { &mut *(func_ref.as_ptr() as *mut Function) }.set_env(Some(env));
}

fn raw_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => {
            // SAFETY: both strings are live Value operands during this C function call.
            let a = unsafe { a.as_ref() }.map(|s| s.data());
            // SAFETY: both strings are live Value operands during this C function call.
            let b = unsafe { b.as_ref() }.map(|s| s.data());
            a == b
        }
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

    // SAFETY: table argument is on the active Lua stack.
    if let Some(table) = unsafe { table_ref.as_ref() }
        && let Some((next_key, next_value)) = table.next(&key)
    {
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
    let Some(next_func) = create_c_function(l, lua_b_next_raw) else {
        return -1;
    };

    l.push_value(Value::Function(next_func));
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
    let Some(iter_func) = create_c_function(l, lua_b_ipairs_iter_raw) else {
        return -1;
    };

    l.push_value(Value::Function(iter_func));
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

    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return 0;
    };
    let next_value = table.get_array(next_index);
    if next_value.is_nil() {
        return 0;
    }

    l.push_value(Value::Number(next_index as f64));
    l.push_value(next_value);
    2
}

fn create_c_function(
    l: &mut LuaState,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) -> Option<GcRef<Function>> {
    let gc_ptr = l.gc?;
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    Some(gc.create(Function::new_c(func)))
}

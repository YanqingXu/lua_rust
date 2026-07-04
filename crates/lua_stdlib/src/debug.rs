//! Minimal debug library.

use std::collections::BTreeSet;

use lua_compiler::opcode::{self, OpCode};
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::table::Table;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use lua_vm::state::{LuaState, Stack};

struct DebugInfo {
    source: String,
    short_src: String,
    what: String,
    currentline: Option<i32>,
    linedefined: Option<i32>,
    lastlinedefined: Option<i32>,
    name: Option<String>,
    namewhat: String,
    func: Option<Value>,
    nups: Option<i32>,
    active_lines: Vec<i32>,
}

struct DebugName {
    name: String,
    namewhat: String,
}

enum ResolvedFrame {
    Real(usize),
    Tail,
}

pub fn open_debug(l: &mut LuaState, gc: &mut GarbageCollector) {
    let debug_table = find_lib_table(l, "debug");
    if debug_table.is_null() {
        return;
    }

    let table_ptr = debug_table.as_ptr() as *mut Table;
    reg(gc, table_ptr, "getfenv", lua_debug_getfenv);
    reg(gc, table_ptr, "gethook", lua_debug_gethook);
    reg(gc, table_ptr, "getinfo", lua_debug_getinfo);
    reg(gc, table_ptr, "getlocal", lua_debug_getlocal);
    reg(gc, table_ptr, "getregistry", lua_debug_getregistry);
    reg(gc, table_ptr, "getupvalue", lua_debug_getupvalue);
    reg(gc, table_ptr, "setfenv", lua_debug_setfenv);
    reg(gc, table_ptr, "sethook", lua_debug_sethook);
    reg(gc, table_ptr, "setlocal", lua_debug_setlocal);
    reg(gc, table_ptr, "setmetatable", lua_debug_setmetatable);
    reg(gc, table_ptr, "setupvalue", lua_debug_setupvalue);
    reg(gc, table_ptr, "traceback", lua_debug_traceback);
}

fn reg(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table points to the library table created and rooted by open_library.
    unsafe {
        (*table).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

fn find_lib_table(l: &LuaState, name: &str) -> GcRef<Table> {
    if let Some(gt) = l.global_table
        // SAFETY: global table is rooted for the duration of library init.
        && let Some(gt_obj) = unsafe { gt.as_ref() }
    {
        for (key, val) in gt_obj.hash_entries() {
            if let Value::String(key_ref) = key
                // SAFETY: key is held by the rooted global table.
                && let Some(key_str) = unsafe { key_ref.as_ref() }
                && key_str.data() == name
                && let Value::Table(t) = val
            {
                return *t;
            }
        }
    }
    GcRef::null()
}

unsafe extern "C" fn lua_debug_getinfo(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    if name_only_options(debug_options(l, 2).as_deref()) {
        return match l.at(1).cloned().unwrap_or(Value::Nil) {
            Value::Function(_) => push_name_info(l, gc, None, String::new()),
            Value::Number(level) if level >= 0.0 => {
                push_name_info_for_level(l, gc, level as usize, false)
            }
            _ => {
                l.push_nil();
                1
            }
        };
    }

    let debug_info = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Function(func_ref) => function_info(func_ref)
            .unwrap_or_else(|| c_debug_info(Some(Value::Function(func_ref)), None)),
        Value::Thread(thread_ref) => {
            let level = match l.at(2).cloned().unwrap_or(Value::Nil) {
                Value::Number(level) if level >= 1.0 => level as usize,
                _ => {
                    l.push_nil();
                    return 1;
                }
            };
            let Some(target) = thread_state_mut(thread_ref) else {
                l.push_nil();
                return 1;
            };
            if name_only_options(debug_options(l, 3).as_deref()) {
                return push_name_info_for_level(target, gc, level, true);
            }
            match stack_frame_info(target, level, true) {
                Some(info) => info,
                None => {
                    l.push_nil();
                    return 1;
                }
            }
        }
        Value::Number(level) if level >= 0.0 => match stack_frame_info(l, level as usize, false) {
            Some(info) => info,
            None if (level as usize) == 1 => DebugInfo {
                source: "?".to_string(),
                short_src: "?".to_string(),
                what: "main".to_string(),
                currentline: Some(0),
                linedefined: None,
                lastlinedefined: None,
                name: None,
                namewhat: String::new(),
                func: None,
                nups: None,
                active_lines: Vec::new(),
            },
            None => {
                l.push_nil();
                return 1;
            }
        },
        _ => {
            l.push_nil();
            return 1;
        }
    };

    let mut info = Table::new();
    set_string_field(&mut info, gc, "source", &debug_info.source);
    set_string_field(&mut info, gc, "short_src", &debug_info.short_src);
    set_string_field(&mut info, gc, "what", &debug_info.what);
    if let Some(line) = debug_info.currentline {
        set_number_field(&mut info, gc, "currentline", line as f64);
    }
    if let Some(line) = debug_info.linedefined {
        set_number_field(&mut info, gc, "linedefined", line as f64);
    }
    if let Some(line) = debug_info.lastlinedefined {
        set_number_field(&mut info, gc, "lastlinedefined", line as f64);
    }
    if let Some(name) = debug_info.name {
        set_string_field(&mut info, gc, "name", &name);
    }
    set_string_field(&mut info, gc, "namewhat", &debug_info.namewhat);
    if let Some(func) = debug_info.func {
        set_value_field(&mut info, gc, "func", func);
    }
    if let Some(nups) = debug_info.nups {
        set_number_field(&mut info, gc, "nups", nups as f64);
    }
    if !debug_info.active_lines.is_empty() {
        let activelines = active_lines_table(gc, &debug_info.active_lines);
        set_value_field(&mut info, gc, "activelines", Value::Table(activelines));
    }

    let info_ref = gc.create(info);
    l.push_value(Value::Table(info_ref));
    1
}

fn debug_options(l: &LuaState, idx: i32) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(options_ref)) => {
            // SAFETY: the option string is an active argument while getinfo runs.
            unsafe { options_ref.as_ref() }.map(|options| options.data().to_string())
        }
        _ => None,
    }
}

fn name_only_options(options: Option<&str>) -> bool {
    options.is_some_and(|options| !options.is_empty() && options.chars().all(|ch| ch == 'n'))
}

fn push_name_info_for_level(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    level: usize,
    include_current: bool,
) -> i32 {
    let frame_idx = match resolve_debug_frame(l, level, include_current) {
        Some(ResolvedFrame::Real(frame_idx)) => frame_idx,
        Some(ResolvedFrame::Tail) => return push_name_info(l, gc, None, String::new()),
        None => {
            l.push_nil();
            return 1;
        }
    };
    let func_ref = {
        let Some(ci) = l.call_stack.get(frame_idx) else {
            l.push_nil();
            return 1;
        };
        frame_function_ref(l, ci)
    };
    let (name, namewhat) = debug_name_for_frame_cached(l, frame_idx, func_ref);
    push_name_info(l, gc, name, namewhat)
}

fn debug_name_for_frame_cached(
    l: &mut LuaState,
    frame_idx: usize,
    func_ref: Option<GcRef<Function>>,
) -> (Option<String>, String) {
    let cache_key = debug_name_cache_key(l, frame_idx, func_ref);
    if let Some(key) = cache_key
        && let Some(cached) = l.debug_name_cache.get(&key)
    {
        return cached.clone();
    }

    let resolved = debug_name_for_frame(l, frame_idx, func_ref);
    if let Some(key) = cache_key {
        l.debug_name_cache.insert(key, resolved.clone());
    }
    resolved
}

fn debug_name_cache_key(
    l: &LuaState,
    frame_idx: usize,
    func_ref: Option<GcRef<Function>>,
) -> Option<(usize, usize, usize)> {
    let caller_idx = frame_idx.checked_sub(1)?;
    let caller_ci = l.call_stack.get(caller_idx)?;
    let pc = caller_ci.savedpc?;
    let caller_proto = frame_proto_ptr(l, caller_ci)? as usize;
    let func_key = func_ref
        .map(|func_ref| func_ref.as_ptr() as usize)
        .unwrap_or(0);
    Some((caller_proto, pc, func_key))
}

fn push_name_info(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    name: Option<String>,
    namewhat: String,
) -> i32 {
    let mut info = Table::new();
    if let Some(name) = name {
        set_string_field(&mut info, gc, "name", &name);
    }
    set_string_field(&mut info, gc, "namewhat", &namewhat);
    let info_ref = gc.create(info);
    l.push_value(Value::Table(info_ref));
    1
}

unsafe extern "C" fn lua_debug_setupvalue(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let Some(Value::Function(func_ref)) = l.at(1).cloned() else {
        l.push_nil();
        return 1;
    };
    let index = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(n) if n >= 1.0 => n as usize - 1,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let value = l.at(3).cloned().unwrap_or(Value::Nil);

    let Some(name) = upvalue_name(func_ref, index) else {
        l.push_nil();
        return 1;
    };
    let Some(upvalue_ref) = function_upvalue(func_ref, index) else {
        l.push_nil();
        return 1;
    };
    set_upvalue_value(l, upvalue_ref, value);

    let name_ref = gc.create(GcString::new(&name));
    l.push_value(Value::String(name_ref));
    1
}

unsafe extern "C" fn lua_debug_getupvalue(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let Some(Value::Function(func_ref)) = l.at(1).cloned() else {
        l.push_nil();
        return 1;
    };
    let index = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(n) if n >= 1.0 => n as usize - 1,
        _ => {
            l.push_nil();
            return 1;
        }
    };

    let Some(name) = upvalue_name(func_ref, index) else {
        l.push_nil();
        return 1;
    };
    let Some(upvalue_ref) = function_upvalue(func_ref, index) else {
        l.push_nil();
        return 1;
    };

    let name_ref = gc.create(GcString::new(&name));
    l.push_value(Value::String(name_ref));
    l.push_value(get_upvalue_value(l, upvalue_ref));
    2
}

unsafe extern "C" fn lua_debug_getregistry(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let table_ref = gc.create(Table::new());
    l.push_value(Value::Table(table_ref));
    1
}

unsafe extern "C" fn lua_debug_traceback(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    if let Some(Value::Thread(thread_ref)) = l.at(1).cloned() {
        let traceback = thread_traceback(thread_ref);
        let message_ref = gc.create(GcString::new(&traceback));
        l.push_value(Value::String(message_ref));
        return 1;
    }

    let (message, has_message) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::String(message_ref) => (
            // SAFETY: traceback argument is on the active Lua stack.
            unsafe { message_ref.as_ref() }
                .map(|message| message.data().to_string())
                .unwrap_or_default(),
            true,
        ),
        Value::Nil => (String::new(), false),
        other => {
            l.push_value(other);
            return 1;
        }
    };
    let level = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value.is_finite() => value as i32,
        _ => 1,
    };
    let mut traceback = if has_message {
        format!("{message}\nstack traceback:\n")
    } else {
        "stack traceback:\n".to_string()
    };
    if level <= 0 {
        traceback.push_str("\t[C]: in function 'traceback'\n");
    }
    let message_ref = gc.create(GcString::new(&traceback));
    l.push_value(Value::String(message_ref));
    1
}

fn thread_traceback(thread_ref: GcRef<Thread>) -> String {
    let Some(state) = thread_state_mut(thread_ref) else {
        return "stack traceback:\n".to_string();
    };
    let mut out = String::from("stack traceback:\n");
    if state.last_error.is_some() {
        out.push_str("\t[C]: in function 'error'\n");
    } else if state.status == lua_vm::state::ThreadStatus::Yield {
        out.push_str("\t[C]: in function 'yield'\n");
    }

    let mut duplicate_first_error_frame = state.last_error.is_some();
    let mut idx = state.current_ci + 1;
    while idx > 0 {
        idx -= 1;
        let Some(ci) = state.call_stack.get(idx) else {
            continue;
        };
        let Some(proto_ptr) = frame_proto_ptr(state, ci) else {
            continue;
        };
        // SAFETY: frame proto pointers are installed by the VM while the frame is live.
        let Some(proto) = (unsafe { proto_ptr.as_ref() }) else {
            continue;
        };
        let func_ref = frame_function_ref(state, ci);
        let source = proto_source(proto);
        let short = short_source(&source);
        let line = ci.savedpc.map(|pc| proto.line(pc)).unwrap_or(0);
        if line == 0 {
            continue;
        }
        let (call_name, _) = debug_name_for_frame(state, idx, func_ref);
        let frame_line = if let Some(name) = call_name.or_else(|| {
            func_ref.and_then(|func_ref| {
                function_name_in_env(state, func_ref)
                    .or_else(|| function_name_in_global(state, func_ref))
            })
        }) {
            format!("\t{short}:{line}: in function '{name}'\n")
        } else {
            format!(
                "\t{short}:{line}: in function <{}:{}>\n",
                short,
                proto.line_defined()
            )
        };
        out.push_str(&frame_line);
        if duplicate_first_error_frame {
            out.push_str(&frame_line);
            duplicate_first_error_frame = false;
        }
        for _ in 0..ci.tailcalls {
            out.push_str("\t(tail call)\n");
        }
    }
    out
}

unsafe extern "C" fn lua_debug_sethook(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let (target_ptr, hook_arg, mask_arg, count_arg) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Thread(thread_ref) => {
            let Some(target) = thread_state_mut(thread_ref) else {
                return 0;
            };
            (target as *mut LuaState, 2, 3, 4)
        }
        _ => (l as *mut LuaState, 1, 2, 3),
    };
    // SAFETY: target_ptr is either l or a coroutine LuaState owned by a live Thread.
    let target = unsafe { &mut *target_ptr };
    let hook = l.at(hook_arg).cloned().unwrap_or(Value::Nil);
    if matches!(hook, Value::Nil) {
        clear_hook(target);
        return 0;
    }

    let mask = match l.at(mask_arg).cloned().unwrap_or(Value::Nil) {
        Value::String(mask_ref) => {
            // SAFETY: mask is an active function argument.
            unsafe { mask_ref.as_ref() }
                .map(|mask| mask.data().to_string())
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    let count = match l.at(count_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value.is_finite() && value > 0.0 => {
            value.min(i32::MAX as f64) as i32
        }
        _ => 0,
    };

    target.debug_hook = Some(hook);
    target.debug_hook_mask = mask;
    target.debug_hook_count = count;
    target.debug_hook_countdown = count;
    target.debug_hook_active = false;
    if target_ptr == l as *mut LuaState {
        let (line, pc, proto) = current_caller_location(l).unwrap_or((-1, usize::MAX, None));
        target.debug_hook_last_line = line;
        target.debug_hook_last_pc = pc;
        target.debug_hook_skip_line = line;
        target.debug_hook_skip_proto = proto;
    } else {
        target.debug_hook_last_line = -1;
        target.debug_hook_last_pc = usize::MAX;
        target.debug_hook_skip_line = -1;
        target.debug_hook_skip_proto = None;
    }
    0
}

unsafe extern "C" fn lua_debug_gethook(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let target_ptr = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Thread(thread_ref) => thread_state_mut(thread_ref)
            .map(|target| target as *mut LuaState)
            .unwrap_or(l as *mut LuaState),
        _ => l as *mut LuaState,
    };
    // SAFETY: target_ptr is either l or a coroutine LuaState owned by a live Thread.
    let target = unsafe { &mut *target_ptr };
    let Some(hook) = target.debug_hook.clone() else {
        l.push_nil();
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let mask_ref = gc.create(GcString::new(&target.debug_hook_mask));
    l.push_value(hook);
    l.push_value(Value::String(mask_ref));
    l.push_value(Value::Number(target.debug_hook_count as f64));
    3
}

fn clear_hook(l: &mut LuaState) {
    l.debug_hook = None;
    l.debug_hook_mask.clear();
    l.debug_hook_count = 0;
    l.debug_hook_countdown = 0;
    l.debug_hook_active = false;
    l.debug_hook_last_line = -1;
    l.debug_hook_last_pc = usize::MAX;
    l.debug_hook_skip_proto = None;
    l.debug_hook_skip_line = -1;
}

fn current_caller_location(l: &LuaState) -> Option<(i32, usize, Option<*const Proto>)> {
    let frame_idx = l.current_ci.checked_sub(1)?;
    let ci = l.call_stack.get(frame_idx)?;
    let pc = ci.savedpc?;
    let Value::Function(func_ref) = l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) else {
        if frame_idx == 0 {
            // SAFETY: current_proto is installed by the VM for top-level chunks.
            return l
                .current_proto
                .and_then(|proto| unsafe { proto.as_ref() })
                .map(|p| (p.line(pc), pc, l.current_proto));
        }
        return None;
    };
    // SAFETY: caller function is held by a live call frame.
    let func = unsafe { func_ref.as_ref() }?;
    let proto_ref = func.proto()?;
    // SAFETY: function keeps its Proto alive.
    unsafe { proto_ref.as_ref() }.map(|proto| (proto.line(pc), pc, Some(proto as *const Proto)))
}

unsafe extern "C" fn lua_debug_getfenv(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let env = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Function(func_ref) => function_env(func_ref).or(l.global_table),
        Value::Thread(thread_ref) => thread_env(thread_ref).or(l.global_table),
        _ => None,
    };
    if let Some(env) = env {
        l.push_value(Value::Table(env));
    } else {
        l.push_nil();
    }
    1
}

unsafe extern "C" fn lua_debug_getlocal(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let (target_ptr, level_arg, local_arg) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Thread(thread_ref) => {
            let Some(target) = thread_state_mut(thread_ref) else {
                l.push_nil();
                return 1;
            };
            (target as *mut LuaState, 2, 3)
        }
        _ => (l as *mut LuaState, 1, 2),
    };
    // SAFETY: target_ptr is either l or a coroutine LuaState owned by a live Thread.
    let target = unsafe { &mut *target_ptr };
    let include_current = target_ptr != l as *mut LuaState;

    let level = match l.at(level_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value >= 0.0 => value as usize,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let local_number = match l.at(local_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value >= 1.0 => value as i32,
        _ => {
            l.push_nil();
            return 1;
        }
    };

    let Some((name, value)) = get_local_value(target, level, local_number, include_current) else {
        l.push_nil();
        return 1;
    };
    let name_ref = gc.create(GcString::new(&name));
    l.push_value(Value::String(name_ref));
    l.push_value(value);
    2
}

unsafe extern "C" fn lua_debug_setlocal(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let (target_ptr, level_arg, local_arg, value_arg) = match l.at(1).cloned().unwrap_or(Value::Nil)
    {
        Value::Thread(thread_ref) => {
            let Some(target) = thread_state_mut(thread_ref) else {
                l.push_nil();
                return 1;
            };
            (target as *mut LuaState, 2, 3, 4)
        }
        _ => (l as *mut LuaState, 1, 2, 3),
    };
    // SAFETY: target_ptr is either l or a coroutine LuaState owned by a live Thread.
    let target = unsafe { &mut *target_ptr };
    let include_current = target_ptr != l as *mut LuaState;

    let level = match l.at(level_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value >= 0.0 => value as usize,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let local_number = match l.at(local_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value >= 1.0 => value as i32,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let value = l.at(value_arg).cloned().unwrap_or(Value::Nil);

    let Some(name) = set_local_value(target, level, local_number, value, include_current) else {
        l.push_nil();
        return 1;
    };
    let name_ref = gc.create(GcString::new(&name));
    l.push_value(Value::String(name_ref));
    1
}

unsafe extern "C" fn lua_debug_setfenv(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let target = l.at(1).cloned().unwrap_or(Value::Nil);
    let Some(Value::Table(env)) = l.at(2).cloned() else {
        l.push_nil();
        return 1;
    };

    match target.clone() {
        Value::Function(func_ref) => set_function_env(func_ref, env),
        Value::Thread(thread_ref) => {
            let Some(state) = thread_state_mut(thread_ref) else {
                l.push_nil();
                return 1;
            };
            state.thread_env = Some(env);
        }
        _ => {
            l.push_nil();
            return 1;
        }
    }

    l.push_value(target);
    1
}

unsafe extern "C" fn lua_debug_setmetatable(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let target = l.at(1).cloned().unwrap_or(Value::Nil);
    let metatable = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Nil => None,
        Value::Table(table_ref) => Some(table_ref),
        _ => {
            l.push_nil();
            return 1;
        }
    };

    match target.clone() {
        Value::Table(table_ref) => {
            // SAFETY: target is an active argument.
            if let Some(table) = unsafe { (table_ref.as_ptr() as *mut Table).as_mut() } {
                table.set_metatable(metatable);
            }
        }
        Value::Userdata(userdata_ref) => {
            if let Some(userdata) =
                // SAFETY: target is an active argument and GC does not run while
                // this C function mutates the userdata metatable.
                unsafe {
                    (userdata_ref.as_ptr() as *mut lua_core::userdata::Userdata).as_mut()
                }
            {
                userdata.set_metatable(metatable);
            }
        }
        Value::Nil => l.nil_metatable = metatable,
        Value::Boolean(_) => l.boolean_metatable = metatable,
        Value::Number(_) => l.number_metatable = metatable,
        _ => {}
    }

    l.push_value(target);
    1
}

fn function_info(func_ref: GcRef<Function>) -> Option<DebugInfo> {
    // SAFETY: function argument is on the active Lua stack.
    let func = unsafe { func_ref.as_ref() }?;
    if func.is_c_function() {
        return Some(c_debug_info(Some(Value::Function(func_ref)), None));
    }
    let proto_ref = func.proto()?;
    // SAFETY: the function keeps its proto alive.
    let proto = unsafe { proto_ref.as_ref() }?;
    let source = proto
        .source()
        .and_then(|source_ref| {
            // SAFETY: the proto keeps its source string alive.
            unsafe { source_ref.as_ref() }.map(|source| source.data().to_string())
        })
        .unwrap_or_default();
    let short_src = short_source(&source);
    Some(DebugInfo {
        source,
        short_src,
        what: "Lua".to_string(),
        currentline: None,
        linedefined: Some(proto.line_defined()),
        lastlinedefined: Some(proto.last_line_defined()),
        name: None,
        namewhat: String::new(),
        func: Some(Value::Function(func_ref)),
        nups: Some(func.num_upvalues() as i32),
        active_lines: active_lines(proto),
    })
}

fn stack_frame_info(l: &LuaState, level: usize, include_current: bool) -> Option<DebugInfo> {
    let frame_idx = match resolve_debug_frame(l, level, include_current)? {
        ResolvedFrame::Real(frame_idx) => frame_idx,
        ResolvedFrame::Tail => return Some(tail_debug_info()),
    };
    let ci = l.call_stack.get(frame_idx)?;
    let func_ref = frame_function_ref(l, ci);
    let is_c_function = func_ref
        .and_then(|func_ref| {
            // SAFETY: validated function refs are held by live call frame slots.
            unsafe { func_ref.as_ref() }.map(|func| func.is_c_function())
        })
        .unwrap_or(false);
    let (name, namewhat) = debug_name_for_frame(l, frame_idx, func_ref);
    if is_c_function && let Some(func_ref) = func_ref {
        let mut info = c_debug_info(Some(Value::Function(func_ref)), name);
        info.namewhat = namewhat;
        return Some(info);
    }
    let proto_ptr = frame_proto_ptr(l, ci)?;
    // SAFETY: frame proto pointers are installed by the VM while the frame is live.
    let proto = unsafe { proto_ptr.as_ref() }?;
    let source = proto
        .source()
        .and_then(|source_ref| {
            // SAFETY: the proto keeps its source string alive.
            unsafe { source_ref.as_ref() }.map(|source| source.data().to_string())
        })
        .unwrap_or_default();
    let line = ci.savedpc.map(|pc| proto.line(pc));
    if include_current && (ci.savedpc.is_none() || line == Some(0)) {
        return None;
    }
    let short_src = short_source(&source);
    Some(DebugInfo {
        source,
        short_src,
        what: if proto.line_defined() == 0 {
            "main".to_string()
        } else {
            "Lua".to_string()
        },
        currentline: line,
        linedefined: Some(proto.line_defined()),
        lastlinedefined: Some(proto.last_line_defined()),
        name,
        namewhat,
        func: func_ref.map(Value::Function),
        nups: Some(
            func_ref
                .and_then(|func_ref| {
                    // SAFETY: validated function refs are held by live call frame slots.
                    unsafe { func_ref.as_ref() }.map(|func| func.num_upvalues() as i32)
                })
                .unwrap_or(0),
        ),
        active_lines: active_lines(proto),
    })
}

fn resolve_debug_frame(l: &LuaState, level: usize, include_current: bool) -> Option<ResolvedFrame> {
    if level == 0 {
        return None;
    }
    let mut remaining = level;
    let mut idx = if include_current {
        l.current_ci + 1
    } else {
        l.current_ci
    };
    while idx > 0 {
        idx -= 1;
        if remaining == 1 {
            return Some(ResolvedFrame::Real(idx));
        }
        remaining -= 1;

        let tailcalls = l.call_stack.get(idx).map(|ci| ci.tailcalls).unwrap_or(0);
        if tailcalls > 0 {
            let tailcalls = tailcalls as usize;
            if remaining <= tailcalls {
                return Some(ResolvedFrame::Tail);
            }
            remaining -= tailcalls;
        }
    }
    None
}

fn tail_debug_info() -> DebugInfo {
    DebugInfo {
        source: "=(tail call)".to_string(),
        short_src: "(tail call)".to_string(),
        what: "tail".to_string(),
        currentline: None,
        linedefined: Some(-1),
        lastlinedefined: Some(-1),
        name: None,
        namewhat: String::new(),
        func: None,
        nups: None,
        active_lines: Vec::new(),
    }
}

fn c_debug_info(func: Option<Value>, name: Option<String>) -> DebugInfo {
    let namewhat = if name.is_some() { "global" } else { "" }.to_string();
    DebugInfo {
        source: "=[C]".to_string(),
        short_src: "[C]".to_string(),
        what: "C".to_string(),
        currentline: None,
        linedefined: None,
        lastlinedefined: None,
        name,
        namewhat,
        func,
        nups: Some(0),
        active_lines: Vec::new(),
    }
}

fn get_local_value(
    l: &LuaState,
    level: usize,
    local_number: i32,
    include_current: bool,
) -> Option<(String, Value)> {
    let frame_idx = frame_index_for_level(l, level, include_current)?;
    if level == 0 {
        return get_temporary(l, frame_idx, local_number, None);
    }

    let ci = l.call_stack.get(frame_idx)?;
    let (proto_ptr, pc) = frame_proto_and_pc(l, frame_idx)?;
    // SAFETY: frame_proto_and_pc only returns pointers owned by live call frames.
    let proto = unsafe { proto_ptr.as_ref() }?;
    if let Some(loc) = proto.local_var_info(local_number, pc as i32) {
        let name = loc.varname.and_then(gc_string_data)?;
        let value = l
            .stack
            .at(ci.base + loc.reg as usize)
            .cloned()
            .unwrap_or(Value::Nil);
        return Some((name, value));
    }

    let temp_number = active_named_local_count(proto, pc) + 1;
    get_temporary(l, frame_idx, local_number, Some(temp_number))
}

fn set_local_value(
    l: &mut LuaState,
    level: usize,
    local_number: i32,
    value: Value,
    include_current: bool,
) -> Option<String> {
    let frame_idx = frame_index_for_level(l, level, include_current)?;
    if level == 0 {
        return set_temporary(l, frame_idx, local_number, value, None);
    }

    let (proto_ptr, pc) = frame_proto_and_pc(l, frame_idx)?;
    // SAFETY: frame_proto_and_pc only returns pointers owned by live call frames.
    let proto = unsafe { proto_ptr.as_ref() }?;
    if let Some(loc) = proto.local_var_info(local_number, pc as i32) {
        let name = loc.varname.and_then(gc_string_data)?;
        let slot = l.call_stack.get(frame_idx)?.base + loc.reg as usize;
        if let Some(dst) = l.stack.at_mut(slot) {
            *dst = value;
            return Some(name);
        }
        return None;
    }

    let temp_number = active_named_local_count(proto, pc) + 1;
    set_temporary(l, frame_idx, local_number, value, Some(temp_number))
}

fn get_temporary(
    l: &LuaState,
    frame_idx: usize,
    local_number: i32,
    only_number: Option<i32>,
) -> Option<(String, Value)> {
    if local_number < 1 {
        return None;
    }
    if let Some(only_number) = only_number
        && local_number != only_number
    {
        return None;
    }
    let ci = l.call_stack.get(frame_idx)?;
    let slot = ci.base + local_number as usize - 1;
    let upper = if frame_idx == l.current_ci {
        l.top
    } else {
        ci.top.min(l.stack.size())
    };
    if slot >= upper {
        return None;
    }
    let value = l.stack.at(slot).cloned().unwrap_or(Value::Nil);
    if matches!(value, Value::Nil) {
        return None;
    }
    Some(("(*temporary)".to_string(), value))
}

fn set_temporary(
    l: &mut LuaState,
    frame_idx: usize,
    local_number: i32,
    value: Value,
    only_number: Option<i32>,
) -> Option<String> {
    if local_number < 1 {
        return None;
    }
    if let Some(only_number) = only_number
        && local_number != only_number
    {
        return None;
    }
    let ci = l.call_stack.get(frame_idx)?;
    let slot = ci.base + local_number as usize - 1;
    let upper = if frame_idx == l.current_ci {
        l.top
    } else {
        ci.top.min(l.stack.size())
    };
    if slot >= upper {
        return None;
    }
    if matches!(l.stack.at(slot), Some(Value::Nil) | None) {
        return None;
    }
    if let Some(dst) = l.stack.at_mut(slot) {
        *dst = value;
        return Some("(*temporary)".to_string());
    }
    None
}

fn active_named_local_count(proto: &Proto, pc: usize) -> i32 {
    let mut count = 0;
    loop {
        let next = count + 1;
        if proto.local_var_info(next, pc as i32).is_some() {
            count = next;
        } else {
            return count;
        }
    }
}

fn frame_index_for_level(l: &LuaState, level: usize, include_current: bool) -> Option<usize> {
    if include_current {
        if level == 0 || level > l.current_ci + 1 {
            None
        } else {
            Some(l.current_ci + 1 - level)
        }
    } else if level > l.current_ci {
        None
    } else {
        Some(l.current_ci - level)
    }
}

fn frame_proto_and_pc(l: &LuaState, frame_idx: usize) -> Option<(*const Proto, usize)> {
    let ci = l.call_stack.get(frame_idx)?;
    let pc = ci.savedpc.unwrap_or(0);
    frame_proto_ptr(l, ci).map(|proto| (proto, pc))
}

fn frame_function_ref(l: &LuaState, ci: &lua_vm::state::CallInfo) -> Option<GcRef<Function>> {
    if ci.func == ci.base && ci.proto.is_some() {
        return None;
    }

    let Value::Function(func_ref) = l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) else {
        return None;
    };

    if let Some(frame_proto) = ci.proto {
        // SAFETY: function refs read from active stack slots stay live during this call.
        let func = unsafe { func_ref.as_ref() }?;
        let func_proto = func.proto()?;
        if func_proto.as_ptr() as *const Proto != frame_proto {
            return None;
        }
    }

    Some(func_ref)
}

fn frame_proto_ptr(l: &LuaState, ci: &lua_vm::state::CallInfo) -> Option<*const Proto> {
    if let Some(proto) = ci.proto {
        return Some(proto);
    }
    let func_ref = frame_function_ref(l, ci)?;
    // SAFETY: validated function refs are held by live call frame slots.
    let func = unsafe { func_ref.as_ref() }?;
    func.proto().map(|proto| proto.as_ptr() as *const Proto)
}

fn active_lines(proto: &lua_core::proto::Proto) -> Vec<i32> {
    let mut lines = BTreeSet::new();
    let first = proto.line_defined() + 1;
    let last = proto.last_line_defined();
    for line in proto.line_info() {
        if *line >= first && (last == 0 || *line <= last) {
            lines.insert(*line);
        }
    }
    if last >= first {
        lines.insert(first);
        lines.insert(last);
    }
    lines.into_iter().collect()
}

fn proto_source(proto: &Proto) -> String {
    proto
        .source()
        .and_then(|source_ref| {
            // SAFETY: the proto keeps its source string alive.
            unsafe { source_ref.as_ref() }.map(|source| source.data().to_string())
        })
        .unwrap_or_default()
}

fn debug_name_for_frame(
    l: &LuaState,
    frame_idx: usize,
    func_ref: Option<GcRef<Function>>,
) -> (Option<String>, String) {
    if let Some(name) = call_site_name(l, frame_idx) {
        return (Some(name.name), name.namewhat);
    }

    let name = func_ref.and_then(|func_ref| function_name_in_env(l, func_ref));
    let namewhat = if name.is_some() { "global" } else { "" }.to_string();
    (name, namewhat)
}

fn call_site_name(l: &LuaState, frame_idx: usize) -> Option<DebugName> {
    let caller_idx = frame_idx.checked_sub(1)?;
    let caller_ci = l.call_stack.get(caller_idx)?;
    let pc = caller_ci.savedpc?;
    let caller_proto_ptr = frame_proto_ptr(l, caller_ci)?;
    // SAFETY: frame proto pointers are installed by the VM while the frame is live.
    let caller_proto = unsafe { caller_proto_ptr.as_ref() }?;
    let call_pc = if pc < caller_proto.instruction_count() {
        pc
    } else {
        let previous = pc.checked_sub(1)?;
        if previous < caller_proto.instruction_count() {
            previous
        } else {
            return None;
        }
    };
    let inst = caller_proto.instruction(call_pc);
    match opcode::get_opcode(inst) {
        OpCode::CALL | OpCode::TAILCALL => {
            let reg = opcode::get_arg_a(inst) as usize;
            register_name(caller_proto, call_pc, reg, 8)
        }
        _ => None,
    }
}

fn register_name(proto: &Proto, pc: usize, reg: usize, depth: usize) -> Option<DebugName> {
    if depth == 0 {
        return None;
    }

    if let Some(name) = local_name_for_reg(proto, reg, pc) {
        return Some(DebugName {
            name,
            namewhat: "local".to_string(),
        });
    }

    for cursor in (0..pc).rev().take(16) {
        let inst = proto.instruction(cursor);
        let op = opcode::get_opcode(inst);
        let a = opcode::get_arg_a(inst) as usize;
        if a != reg {
            continue;
        }

        match op {
            OpCode::MOVE => {
                let source = opcode::get_arg_b(inst) as usize;
                return register_name(proto, cursor, source, depth - 1);
            }
            OpCode::GETUPVAL => {
                let upvalue = opcode::get_arg_b(inst) as usize;
                return proto
                    .upvalue_name(upvalue)
                    .and_then(gc_string_data)
                    .map(|name| DebugName {
                        name,
                        namewhat: "upvalue".to_string(),
                    });
            }
            OpCode::GETGLOBAL => {
                let bx = opcode::get_arg_bx(inst) as usize;
                return constant_string(proto.constants(), bx).map(|name| DebugName {
                    name,
                    namewhat: "global".to_string(),
                });
            }
            OpCode::GETTABLE => {
                let key = opcode::get_arg_c(inst);
                return rk_string(proto.constants(), key).map(|name| DebugName {
                    name,
                    namewhat: "field".to_string(),
                });
            }
            OpCode::SELF => {
                let key = opcode::get_arg_c(inst);
                return rk_string(proto.constants(), key).map(|name| DebugName {
                    name,
                    namewhat: "method".to_string(),
                });
            }
            _ => return None,
        }
    }

    None
}

fn local_name_for_reg(proto: &Proto, reg: usize, pc: usize) -> Option<String> {
    let pc = pc as i32;
    for idx in (0..proto.loc_var_count()).rev() {
        let loc = proto.loc_var(idx);
        if loc.reg == reg as i32
            && loc.startpc <= pc
            && pc < loc.endpc
            && let Some(name_ref) = loc.varname
        {
            return gc_string_data(name_ref);
        }
    }
    None
}

fn rk_string(constants: &[Value], rk: i32) -> Option<String> {
    if opcode::is_k(rk) {
        constant_string(constants, opcode::index_k(rk) as usize)
    } else {
        None
    }
}

fn constant_string(constants: &[Value], idx: usize) -> Option<String> {
    match constants.get(idx) {
        Some(Value::String(name_ref)) => gc_string_data(*name_ref),
        _ => None,
    }
}

fn gc_string_data(name_ref: GcRef<GcString>) -> Option<String> {
    // SAFETY: debug metadata is owned by live Proto/constant tables while executing.
    unsafe { name_ref.as_ref() }.map(|name| name.data().to_string())
}

fn short_source(source: &str) -> String {
    if let Some(stripped) = source.strip_prefix('=') {
        return stripped.to_string();
    }
    if let Some(stripped) = source.strip_prefix('@') {
        return shorten_file_name(stripped);
    }
    if source.starts_with("[string ") {
        return source.to_string();
    }
    string_chunk_id(source)
}

fn shorten_file_name(source: &str) -> String {
    const LUA_IDSIZE: usize = 60;
    if source.chars().count() <= LUA_IDSIZE {
        return source.to_string();
    }
    let tail: String = source
        .chars()
        .rev()
        .take(LUA_IDSIZE - 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}")
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

fn function_name_in_env(l: &LuaState, func_ref: GcRef<Function>) -> Option<String> {
    // SAFETY: the function is held by the active stack frame or an argument.
    let env = unsafe { func_ref.as_ref() }
        .and_then(|function| function.env())
        .or(l.global_table)?;
    // SAFETY: env is reachable from the function/global state while getinfo runs.
    let table = unsafe { env.as_ref() }?;
    for (key, value) in table.hash_entries() {
        if let (Value::String(key_ref), Value::Function(value_func)) = (key, value)
            && *value_func == func_ref
            // SAFETY: key is held by the environment table.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
        {
            return Some(key_string.data().to_string());
        }
    }
    None
}

fn function_name_in_global(l: &LuaState, func_ref: GcRef<Function>) -> Option<String> {
    let env = l.global_table?;
    // SAFETY: global table is rooted by the Lua state while debugging.
    let table = unsafe { env.as_ref() }?;
    for (key, value) in table.hash_entries() {
        if let (Value::String(key_ref), Value::Function(value_func)) = (key, value)
            && *value_func == func_ref
            // SAFETY: key is held by the global table.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
        {
            return Some(key_string.data().to_string());
        }
    }
    None
}

fn set_string_field(table: &mut Table, gc: &mut GarbageCollector, key: &str, value: &str) {
    let key = gc.create(GcString::new(key));
    let value = gc.create(GcString::new(value));
    table.set(&Value::String(key), &Value::String(value));
}

fn set_number_field(table: &mut Table, gc: &mut GarbageCollector, key: &str, value: f64) {
    let key = gc.create(GcString::new(key));
    table.set(&Value::String(key), &Value::Number(value));
}

fn set_value_field(table: &mut Table, gc: &mut GarbageCollector, key: &str, value: Value) {
    let key = gc.create(GcString::new(key));
    table.set(&Value::String(key), &value);
}

fn active_lines_table(gc: &mut GarbageCollector, lines: &[i32]) -> GcRef<Table> {
    let mut table = Table::new();
    for line in lines {
        table.set(&Value::Number(*line as f64), &Value::Boolean(true));
    }
    gc.create(table)
}

fn upvalue_name(func_ref: GcRef<Function>, index: usize) -> Option<String> {
    // SAFETY: function argument is on the active Lua stack.
    let func = unsafe { func_ref.as_ref() }?;
    let proto_ref = func.proto()?;
    // SAFETY: the function keeps its proto alive.
    let proto = unsafe { proto_ref.as_ref() }?;
    let name_ref = proto.upvalue_name(index)?;
    // SAFETY: the proto keeps its upvalue-name string alive.
    unsafe { name_ref.as_ref() }.map(|name| name.data().to_string())
}

fn function_upvalue(
    func_ref: GcRef<Function>,
    index: usize,
) -> Option<GcRef<lua_core::upvalue::Upvalue>> {
    // SAFETY: function argument is on the active Lua stack.
    let func = unsafe { func_ref.as_ref() }?;
    func.upvalue(index)
}

fn get_upvalue_value(l: &LuaState, upvalue_ref: GcRef<Upvalue>) -> Value {
    // SAFETY: upvalue is kept alive by the function being inspected.
    let Some(upvalue) = (unsafe { upvalue_ref.as_ref() }) else {
        return Value::Nil;
    };
    if upvalue.is_open() {
        let owner_stack = upvalue.owner_stack();
        if owner_stack.is_null() {
            l.stack
                .at(upvalue.stack_index())
                .cloned()
                .unwrap_or(Value::Nil)
        } else {
            // SAFETY: open upvalues store the Stack pointer supplied by the owning LuaState.
            let stack = unsafe { &*(owner_stack as *const Stack) };
            stack
                .at(upvalue.stack_index())
                .cloned()
                .unwrap_or(Value::Nil)
        }
    } else {
        upvalue.get_closed_value().clone()
    }
}

fn set_upvalue_value(l: &mut LuaState, upvalue_ref: GcRef<Upvalue>, value: Value) {
    // SAFETY: upvalue is kept alive by the function being inspected.
    let upvalue = unsafe { &mut *(upvalue_ref.as_ptr() as *mut Upvalue) };
    if upvalue.is_open() {
        let owner_stack = upvalue.owner_stack();
        if owner_stack.is_null() {
            if let Some(slot) = l.stack.at_mut(upvalue.stack_index()) {
                *slot = value;
            }
        } else {
            // SAFETY: open upvalues store the Stack pointer supplied by the owning LuaState.
            let stack = unsafe { &mut *(owner_stack as *mut Stack) };
            if let Some(slot) = stack.at_mut(upvalue.stack_index()) {
                *slot = value;
            }
        }
    } else {
        upvalue.set_closed_value(value);
    }
}

fn function_env(func_ref: GcRef<Function>) -> Option<GcRef<Table>> {
    // SAFETY: function refs are held by a Lua stack or GC object.
    unsafe { func_ref.as_ref() }.and_then(|function| function.env())
}

fn set_function_env(func_ref: GcRef<Function>, env: GcRef<Table>) {
    // SAFETY: function refs are held by a Lua stack or GC object.
    unsafe { &mut *(func_ref.as_ptr() as *mut Function) }.set_env(Some(env));
}

fn thread_env(thread_ref: GcRef<Thread>) -> Option<GcRef<Table>> {
    thread_state_mut(thread_ref).and_then(|state| state.thread_env.or(state.global_table))
}

fn thread_state_mut(thread_ref: GcRef<Thread>) -> Option<&'static mut LuaState> {
    if thread_ref.is_null() {
        return None;
    }
    // SAFETY: thread_ref is held by a Lua stack or GC object.
    let thread = unsafe { thread_ref.as_ref() }?;
    let ptr = thread.lua_state() as *mut LuaState;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Thread::lua_state points to the Box<LuaState> installed by coroutine.create.
    Some(unsafe { &mut *ptr })
}

//! Minimal debug library.

use std::collections::BTreeSet;
use std::ptr::NonNull;

use lua_compiler::opcode::{self, OpCode};
use lua_core::function::Function;
use lua_core::gc::collector::{GarbageCollector, GcRefValidationError};
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::publication::{PublicationTxn, Rooted};
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::table::Table;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use lua_vm::state::LuaState;

struct DebugInfo {
    source: Vec<u8>,
    short_src: Vec<u8>,
    what: Vec<u8>,
    currentline: Option<i32>,
    linedefined: Option<i32>,
    lastlinedefined: Option<i32>,
    name: Option<Vec<u8>>,
    namewhat: Vec<u8>,
    func: Option<Value>,
    nups: Option<i32>,
    active_lines: Vec<i32>,
}

struct DebugName {
    name: Vec<u8>,
    namewhat: Vec<u8>,
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

    reg(l, gc, debug_table, "getfenv", lua_debug_getfenv);
    reg(l, gc, debug_table, "gethook", lua_debug_gethook);
    reg(l, gc, debug_table, "getinfo", lua_debug_getinfo);
    reg(l, gc, debug_table, "getlocal", lua_debug_getlocal);
    reg(l, gc, debug_table, "getregistry", lua_debug_getregistry);
    reg(l, gc, debug_table, "getupvalue", lua_debug_getupvalue);
    reg(l, gc, debug_table, "setfenv", lua_debug_setfenv);
    reg(l, gc, debug_table, "sethook", lua_debug_sethook);
    reg(l, gc, debug_table, "setlocal", lua_debug_setlocal);
    reg(l, gc, debug_table, "setmetatable", lua_debug_setmetatable);
    reg(l, gc, debug_table, "setupvalue", lua_debug_setupvalue);
    reg(l, gc, debug_table, "traceback", lua_debug_traceback);
}

fn reg(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), func, None)
        .expect("debug Function publication must remain collector-valid");
}

fn find_lib_table(l: &LuaState, name: &str) -> GcRef<Table> {
    crate::registration::find_library_table(l, name.as_bytes())
        .ok()
        .flatten()
        .unwrap_or_else(GcRef::null)
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
            Value::Function(_) => push_name_info(l, gc, None, Vec::new()),
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
        Value::Function(func_ref) => function_info(gc, func_ref)
            .unwrap_or_else(|| c_debug_info(Some(Value::Function(func_ref)), None)),
        Value::Thread(thread_ref) => {
            let level = match l.at(2).cloned().unwrap_or(Value::Nil) {
                Value::Number(level) if level >= 1.0 => level as usize,
                _ => {
                    l.push_nil();
                    return 1;
                }
            };
            if name_only_options(debug_options(l, 3).as_deref()) {
                let Some(result) = with_thread_state_mut(l, thread_ref, |target| {
                    push_name_info_for_level(target, gc, level, true)
                }) else {
                    l.push_nil();
                    return 1;
                };
                return result;
            }
            match with_thread_state_mut(l, thread_ref, |target| {
                stack_frame_info(target, level, true)
            })
            .flatten()
            {
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
                source: b"?".to_vec(),
                short_src: b"?".to_vec(),
                what: b"main".to_vec(),
                currentline: Some(0),
                linedefined: None,
                lastlinedefined: None,
                name: None,
                namewhat: Vec::new(),
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

    if publish_debug_info(l, gc, debug_info).is_err() {
        l.push_nil();
    }
    1
}

fn debug_options(l: &LuaState, idx: i32) -> Option<Vec<u8>> {
    match l.at(idx) {
        Some(Value::String(options_ref)) => l.copy_string_bytes(*options_ref).ok(),
        _ => None,
    }
}

fn name_only_options(options: Option<&[u8]>) -> bool {
    options.is_some_and(|options| !options.is_empty() && options.iter().all(|byte| *byte == b'n'))
}

fn push_name_info_for_level(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    level: usize,
    include_current: bool,
) -> i32 {
    let frame_idx = match resolve_debug_frame(l, level, include_current) {
        Some(ResolvedFrame::Real(frame_idx)) => frame_idx,
        Some(ResolvedFrame::Tail) => return push_name_info(l, gc, None, Vec::new()),
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
    let (name, namewhat) = debug_name_for_frame(l, frame_idx, func_ref);
    push_name_info(l, gc, name, namewhat)
}

fn push_name_info(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    name: Option<Vec<u8>>,
    namewhat: Vec<u8>,
) -> i32 {
    let publication = gc.with_publication(|transaction| {
        let info = transaction.alloc(Table::new());
        if let Some(name) = name {
            set_string_field(l, transaction, &info, b"name", &name)?;
        }
        set_string_field(l, transaction, &info, b"namewhat", &namewhat)?;
        // SAFETY: the completed Table is installed on the active stack.
        unsafe { transaction.publish_table_value(info, |value| l.push_value(value)) }
    });
    if publication.is_err() {
        l.push_nil();
    }
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

    let Some(name) = upvalue_name(gc, func_ref, index) else {
        l.push_nil();
        return 1;
    };
    let Some(upvalue_ref) = function_upvalue(func_ref, index) else {
        l.push_nil();
        return 1;
    };
    if let Err(message) = set_upvalue_value(l, gc, upvalue_ref, value) {
        let _ = crate::registration::push_string(l, gc, message.as_bytes());
        return -1;
    }

    let _ = crate::registration::push_string(l, gc, &name);
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

    let Some(name) = upvalue_name(gc, func_ref, index) else {
        l.push_nil();
        return 1;
    };
    let Some(upvalue_ref) = function_upvalue(func_ref, index) else {
        l.push_nil();
        return 1;
    };

    let _ = crate::registration::push_string(l, gc, &name);
    let value = match get_upvalue_value(l, gc, upvalue_ref) {
        Ok(value) => value,
        Err(message) => {
            let _ = crate::registration::push_string(l, gc, message.as_bytes());
            return -1;
        }
    };
    l.push_value(value);
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
    let publication = gc.with_publication(|transaction| {
        let table = transaction.alloc(Table::new());
        // SAFETY: the callback installs the Table on the active result stack.
        unsafe { transaction.publish_table_value(table, |value| l.push_value(value)) }
    });
    if publication.is_err() {
        l.push_nil();
    }
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
        let traceback = thread_traceback(l, thread_ref);
        let _ = crate::registration::push_string(l, gc, &traceback);
        return 1;
    }

    let (message, has_message) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::String(message_ref) => (l.copy_string_bytes(message_ref).unwrap_or_default(), true),
        Value::Nil => (Vec::new(), false),
        other => {
            l.push_value(other);
            return 1;
        }
    };
    let level = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value.is_finite() => value as i32,
        _ => 1,
    };
    let mut traceback = message;
    if has_message {
        traceback.push(b'\n');
    }
    traceback.extend_from_slice(b"stack traceback:\n");
    if level <= 0 {
        traceback.extend_from_slice(b"\t[C]: in function 'traceback'\n");
    }
    let _ = crate::registration::push_string(l, gc, &traceback);
    1
}

fn thread_traceback(current: &mut LuaState, thread_ref: GcRef<Thread>) -> Vec<u8> {
    with_thread_state_mut(current, thread_ref, state_traceback)
        .unwrap_or_else(|| b"stack traceback:\n".to_vec())
}

fn state_traceback(state: &mut LuaState) -> Vec<u8> {
    let mut out = b"stack traceback:\n".to_vec();
    if state.last_error.is_some() {
        out.extend_from_slice(b"\t[C]: in function 'error'\n");
    } else if state.status == lua_vm::state::ThreadStatus::Yield {
        out.extend_from_slice(b"\t[C]: in function 'yield'\n");
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
        // SAFETY: frame_proto_ptr validated the managed handle against the
        // owning collector; destructive sweep is disabled during callbacks.
        let proto = unsafe { proto_ptr.as_ref() };
        let func_ref = frame_function_ref(state, ci);
        let source = proto_source(state, proto);
        let short = short_source(&source);
        let line = ci.savedpc.map(|pc| proto.line(pc)).unwrap_or(0);
        if line == 0 {
            continue;
        }
        let (call_name, _) = debug_name_for_frame(state, idx, func_ref);
        let mut frame_line = Vec::new();
        frame_line.push(b'\t');
        frame_line.extend_from_slice(&short);
        frame_line.push(b':');
        frame_line.extend_from_slice(line.to_string().as_bytes());
        if let Some(name) = call_name.or_else(|| {
            func_ref.and_then(|func_ref| {
                function_name_in_env(state, func_ref)
                    .or_else(|| function_name_in_global(state, func_ref))
            })
        }) {
            frame_line.extend_from_slice(b": in function '");
            frame_line.extend_from_slice(&name);
            frame_line.extend_from_slice(b"'\n");
        } else {
            frame_line.extend_from_slice(b": in function <");
            frame_line.extend_from_slice(&short);
            frame_line.push(b':');
            frame_line.extend_from_slice(proto.line_defined().to_string().as_bytes());
            frame_line.extend_from_slice(b">\n");
        }
        out.extend_from_slice(&frame_line);
        if duplicate_first_error_frame {
            out.extend_from_slice(&frame_line);
            duplicate_first_error_frame = false;
        }
        for _ in 0..ci.tailcalls {
            out.extend_from_slice(b"\t(tail call)\n");
        }
    }
    out
}

unsafe extern "C" fn lua_debug_sethook(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let (target_thread, hook_arg, mask_arg, count_arg) =
        match l.at(1).cloned().unwrap_or(Value::Nil) {
            Value::Thread(thread_ref) => (Some(thread_ref), 2, 3, 4),
            _ => (None, 1, 2, 3),
        };
    let hook = l.at(hook_arg).cloned().unwrap_or(Value::Nil);
    if matches!(hook, Value::Nil) {
        if let Some(thread_ref) = target_thread {
            with_thread_state_mut(l, thread_ref, clear_hook);
        } else {
            clear_hook(l);
        }
        return 0;
    }

    let mask = match l.at(mask_arg).cloned().unwrap_or(Value::Nil) {
        Value::String(mask_ref) => l.copy_string_bytes(mask_ref).unwrap_or_default(),
        _ => Vec::new(),
    };
    let count = match l.at(count_arg).cloned().unwrap_or(Value::Nil) {
        Value::Number(value) if value.is_finite() && value > 0.0 => {
            value.min(i32::MAX as f64) as i32
        }
        _ => 0,
    };

    if let Some(thread_ref) = target_thread {
        with_thread_state_mut(l, thread_ref, |target| {
            install_hook(target, hook, mask, count, None);
        });
        return 0;
    }

    let location = current_caller_location(l).unwrap_or((-1, usize::MAX, None));
    install_hook(l, hook, mask, count, Some(location));
    0
}

fn install_hook(
    target: &mut LuaState,
    hook: Value,
    mask: Vec<u8>,
    count: i32,
    current_location: Option<(i32, usize, Option<GcRef<Proto>>)>,
) {
    target.debug_hook = Some(hook);
    target.debug_hook_mask = mask_bytes_to_storage(&mask);
    target.debug_hook_count = count;
    target.debug_hook_countdown = count;
    target.debug_hook_active = false;
    if let Some((line, pc, proto)) = current_location {
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
}

unsafe extern "C" fn lua_debug_gethook(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let hook_info = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Thread(thread_ref) => {
            with_thread_state_mut(l, thread_ref, hook_snapshot).unwrap_or_else(|| hook_snapshot(l))
        }
        _ => hook_snapshot(l),
    };
    let Some((hook, mask, count)) = hook_info else {
        l.push_nil();
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    l.push_value(hook);
    let _ = crate::registration::push_string(l, gc, &mask);
    l.push_value(Value::Number(count as f64));
    3
}

fn hook_snapshot(target: &mut LuaState) -> Option<(Value, Vec<u8>, i32)> {
    Some((
        target.debug_hook.clone()?,
        mask_storage_to_bytes(&target.debug_hook_mask),
        target.debug_hook_count,
    ))
}

/// Transitional lossless storage bridge for `LuaState::debug_hook_mask`.
///
/// LuaState still exposes this field as `String`; mapping each byte to the
/// same-codepoint char preserves all 256 byte values and keeps the VM's
/// existing ASCII `contains('c'/'r'/'l')` checks exact.
fn mask_bytes_to_storage(mask: &[u8]) -> String {
    mask.iter().copied().map(char::from).collect()
}

fn mask_storage_to_bytes(mask: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(mask.len());
    for ch in mask.chars() {
        if u32::from(ch) <= u32::from(u8::MAX) {
            bytes.push(ch as u8);
        } else {
            let mut encoded = [0_u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
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

fn current_caller_location(l: &LuaState) -> Option<(i32, usize, Option<GcRef<Proto>>)> {
    let frame_idx = l.current_ci.checked_sub(1)?;
    let ci = l.call_stack.get(frame_idx)?;
    let pc = ci.savedpc?;
    let proto_ref = frame_proto_handle(l, ci)?;
    let proto_ptr = validated_proto_ptr(l, proto_ref)?;
    // SAFETY: validated_proto_ptr checked this handle against the state
    // collector, and destructive sweep is disabled during debug callbacks.
    let proto = unsafe { proto_ptr.as_ref() };
    Some((proto.line(pc), pc, Some(proto_ref)))
}

unsafe extern "C" fn lua_debug_getfenv(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let env = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Function(func_ref) => function_env(func_ref).or(l.global_table),
        Value::Thread(thread_ref) => thread_env(l, thread_ref).or(l.global_table),
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
    let (target_thread, level_arg, local_arg) = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Thread(thread_ref) => (Some(thread_ref), 2, 3),
        _ => (None, 1, 2),
    };
    let include_current = target_thread.is_some();

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

    let local = if let Some(thread_ref) = target_thread {
        with_thread_state_mut(l, thread_ref, |target| {
            get_local_value(target, level, local_number, include_current)
        })
        .flatten()
    } else {
        get_local_value(l, level, local_number, include_current)
    };
    let Some((name, value)) = local else {
        l.push_nil();
        return 1;
    };
    let _ = crate::registration::push_string(l, gc, &name);
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
    let (target_thread, level_arg, local_arg, value_arg) =
        match l.at(1).cloned().unwrap_or(Value::Nil) {
            Value::Thread(thread_ref) => (Some(thread_ref), 2, 3, 4),
            _ => (None, 1, 2, 3),
        };
    let include_current = target_thread.is_some();

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

    let name = if let Some(thread_ref) = target_thread {
        with_thread_state_mut(l, thread_ref, |target| {
            set_local_value(target, level, local_number, value, include_current)
        })
        .flatten()
    } else {
        set_local_value(l, level, local_number, value, include_current)
    };
    let Some(name) = name else {
        l.push_nil();
        return 1;
    };
    let _ = crate::registration::push_string(l, gc, &name);
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
            let Some(()) = with_thread_state_mut(l, thread_ref, |state| {
                state.thread_env = Some(env);
            }) else {
                l.push_nil();
                return 1;
            };
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

fn function_info(gc: &GarbageCollector, func_ref: GcRef<Function>) -> Option<DebugInfo> {
    let (is_c_function, proto_ref, num_upvalues) = gc
        .with_ref(func_ref, |func| {
            (
                func.is_c_function(),
                func.proto(),
                func.num_upvalues() as i32,
            )
        })
        .ok()?;
    if is_c_function {
        return Some(c_debug_info(Some(Value::Function(func_ref)), None));
    }
    let proto_ref = proto_ref?;
    let proto_ptr = gc.validate_ref(proto_ref).ok()?;
    // SAFETY: validated above; destructive sweep is disabled throughout this
    // debug C callback.
    let proto = unsafe { proto_ptr.as_ref() };
    let source = proto
        .source()
        .and_then(|source_ref| gc.with_string_bytes(source_ref, <[u8]>::to_vec).ok())
        .unwrap_or_default();
    let short_src = short_source(&source);
    Some(DebugInfo {
        source,
        short_src,
        what: b"Lua".to_vec(),
        currentline: None,
        linedefined: Some(proto.line_defined()),
        lastlinedefined: Some(proto.last_line_defined()),
        name: None,
        namewhat: Vec::new(),
        func: Some(Value::Function(func_ref)),
        nups: Some(num_upvalues),
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
    // SAFETY: frame_proto_ptr validated the managed handle against the owning
    // collector; destructive sweep is disabled during callbacks.
    let proto = unsafe { proto_ptr.as_ref() };
    let source = proto
        .source()
        .and_then(|source_ref| l.copy_string_bytes(source_ref).ok())
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
            b"main".to_vec()
        } else {
            b"Lua".to_vec()
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
        source: b"=(tail call)".to_vec(),
        short_src: b"(tail call)".to_vec(),
        what: b"tail".to_vec(),
        currentline: None,
        linedefined: Some(-1),
        lastlinedefined: Some(-1),
        name: None,
        namewhat: Vec::new(),
        func: None,
        nups: None,
        active_lines: Vec::new(),
    }
}

fn c_debug_info(func: Option<Value>, name: Option<Vec<u8>>) -> DebugInfo {
    let namewhat = if name.is_some() {
        b"global".to_vec()
    } else {
        Vec::new()
    };
    DebugInfo {
        source: b"=[C]".to_vec(),
        short_src: b"[C]".to_vec(),
        what: b"C".to_vec(),
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
) -> Option<(Vec<u8>, Value)> {
    let frame_idx = frame_index_for_level(l, level, include_current)?;
    if level == 0 {
        return get_temporary(l, frame_idx, local_number, None);
    }

    let ci = l.call_stack.get(frame_idx)?;
    let (proto_ptr, pc) = frame_proto_and_pc(l, frame_idx)?;
    // SAFETY: frame_proto_and_pc validated the managed handle against the
    // owning collector.
    let proto = unsafe { proto_ptr.as_ref() };
    if let Some(loc) = proto.local_var_info(local_number, pc as i32) {
        let name = loc.varname.and_then(|name| gc_string_bytes(l, name))?;
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
) -> Option<Vec<u8>> {
    let frame_idx = frame_index_for_level(l, level, include_current)?;
    if level == 0 {
        return set_temporary(l, frame_idx, local_number, value, None);
    }

    let (proto_ptr, pc) = frame_proto_and_pc(l, frame_idx)?;
    // SAFETY: frame_proto_and_pc validated the managed handle against the
    // owning collector.
    let proto = unsafe { proto_ptr.as_ref() };
    if let Some(loc) = proto.local_var_info(local_number, pc as i32) {
        let name = loc.varname.and_then(|name| gc_string_bytes(l, name))?;
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
) -> Option<(Vec<u8>, Value)> {
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
    Some((b"(*temporary)".to_vec(), value))
}

fn set_temporary(
    l: &mut LuaState,
    frame_idx: usize,
    local_number: i32,
    value: Value,
    only_number: Option<i32>,
) -> Option<Vec<u8>> {
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
        return Some(b"(*temporary)".to_vec());
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

fn frame_proto_and_pc(l: &LuaState, frame_idx: usize) -> Option<(NonNull<Proto>, usize)> {
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
        let func_proto = state_gc(l)
            .and_then(|gc| gc.with_ref(func_ref, Function::proto).ok())
            .flatten()?;
        if func_proto != frame_proto {
            return None;
        }
    }

    Some(func_ref)
}

fn frame_proto_handle(l: &LuaState, ci: &lua_vm::state::CallInfo) -> Option<GcRef<Proto>> {
    if let Some(proto) = ci.proto {
        return Some(proto);
    }
    let func_ref = frame_function_ref(l, ci)?;
    state_gc(l)
        .and_then(|gc| gc.with_ref(func_ref, Function::proto).ok())
        .flatten()
}

fn frame_proto_ptr(l: &LuaState, ci: &lua_vm::state::CallInfo) -> Option<NonNull<Proto>> {
    validated_proto_ptr(l, frame_proto_handle(l, ci)?)
}

fn validated_proto_ptr(l: &LuaState, proto: GcRef<Proto>) -> Option<NonNull<Proto>> {
    state_gc(l)?.validate_ref(proto).ok()
}

fn state_gc(l: &LuaState) -> Option<&GarbageCollector> {
    let gc = l.gc?;
    // SAFETY: the VM installs the owning collector for the complete duration
    // of C/debug callbacks. This immutable borrow is scoped to one helper call.
    Some(unsafe { &*gc })
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

fn proto_source(l: &LuaState, proto: &Proto) -> Vec<u8> {
    proto
        .source()
        .and_then(|source_ref| l.copy_string_bytes(source_ref).ok())
        .unwrap_or_default()
}

fn debug_name_for_frame(
    l: &LuaState,
    frame_idx: usize,
    func_ref: Option<GcRef<Function>>,
) -> (Option<Vec<u8>>, Vec<u8>) {
    if let Some(name) = call_site_name(l, frame_idx) {
        return (Some(name.name), name.namewhat);
    }

    let name = func_ref.and_then(|func_ref| function_name_in_env(l, func_ref));
    let namewhat = if name.is_some() {
        b"global".to_vec()
    } else {
        Vec::new()
    };
    (name, namewhat)
}

fn call_site_name(l: &LuaState, frame_idx: usize) -> Option<DebugName> {
    let caller_idx = frame_idx.checked_sub(1)?;
    let caller_ci = l.call_stack.get(caller_idx)?;
    let pc = caller_ci.savedpc?;
    let caller_proto_ptr = frame_proto_ptr(l, caller_ci)?;
    // SAFETY: frame_proto_ptr validated the managed handle against the owning
    // collector.
    let caller_proto = unsafe { caller_proto_ptr.as_ref() };
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
            register_name(l, caller_proto, call_pc, reg, 8)
        }
        _ => None,
    }
}

fn register_name(
    l: &LuaState,
    proto: &Proto,
    pc: usize,
    reg: usize,
    depth: usize,
) -> Option<DebugName> {
    if depth == 0 {
        return None;
    }

    if let Some(name) = local_name_for_reg(l, proto, reg, pc) {
        return Some(DebugName {
            name,
            namewhat: b"local".to_vec(),
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
                return register_name(l, proto, cursor, source, depth - 1);
            }
            OpCode::GETUPVAL => {
                let upvalue = opcode::get_arg_b(inst) as usize;
                return proto
                    .upvalue_name(upvalue)
                    .and_then(|name| gc_string_bytes(l, name))
                    .map(|name| DebugName {
                        name,
                        namewhat: b"upvalue".to_vec(),
                    });
            }
            OpCode::GETGLOBAL => {
                let bx = opcode::get_arg_bx(inst) as usize;
                return constant_string(l, proto.constants(), bx).map(|name| DebugName {
                    name,
                    namewhat: b"global".to_vec(),
                });
            }
            OpCode::GETTABLE => {
                let key = opcode::get_arg_c(inst);
                return rk_string(l, proto.constants(), key).map(|name| DebugName {
                    name,
                    namewhat: b"field".to_vec(),
                });
            }
            OpCode::SELF => {
                let key = opcode::get_arg_c(inst);
                return rk_string(l, proto.constants(), key).map(|name| DebugName {
                    name,
                    namewhat: b"method".to_vec(),
                });
            }
            _ => return None,
        }
    }

    None
}

fn local_name_for_reg(l: &LuaState, proto: &Proto, reg: usize, pc: usize) -> Option<Vec<u8>> {
    let pc = pc as i32;
    for idx in (0..proto.loc_var_count()).rev() {
        let loc = proto.loc_var(idx);
        if loc.reg == reg as i32
            && loc.startpc <= pc
            && pc < loc.endpc
            && let Some(name_ref) = loc.varname
        {
            return gc_string_bytes(l, name_ref);
        }
    }
    None
}

fn rk_string(l: &LuaState, constants: &[Value], rk: i32) -> Option<Vec<u8>> {
    if opcode::is_k(rk) {
        constant_string(l, constants, opcode::index_k(rk) as usize)
    } else {
        None
    }
}

fn constant_string(l: &LuaState, constants: &[Value], idx: usize) -> Option<Vec<u8>> {
    match constants.get(idx) {
        Some(Value::String(name_ref)) => gc_string_bytes(l, *name_ref),
        _ => None,
    }
}

fn gc_string_bytes(l: &LuaState, name_ref: GcRef<GcString>) -> Option<Vec<u8>> {
    l.copy_string_bytes(name_ref).ok()
}

fn short_source(source: &[u8]) -> Vec<u8> {
    if let Some(stripped) = source.strip_prefix(b"=") {
        return stripped.to_vec();
    }
    if let Some(stripped) = source.strip_prefix(b"@") {
        return shorten_file_name(stripped);
    }
    if source.starts_with(b"[string ") {
        return source.to_vec();
    }
    string_chunk_id(source)
}

fn shorten_file_name(source: &[u8]) -> Vec<u8> {
    const LUA_IDSIZE: usize = 60;
    if source.len() <= LUA_IDSIZE {
        return source.to_vec();
    }
    let mut shortened = b"...".to_vec();
    shortened.extend_from_slice(&source[source.len() - (LUA_IDSIZE - 3)..]);
    shortened
}

fn string_chunk_id(source: &[u8]) -> Vec<u8> {
    const LUA_IDSIZE: usize = 60;
    let newline = source.iter().position(|byte| *byte == b'\n');
    let before_newline = newline.map_or(source, |index| &source[..index]);
    if before_newline.is_empty() && newline.is_some() {
        return b"[string \"...\"]".to_vec();
    }

    let needs_ellipsis = newline.is_some() || source.len() > before_newline.len();
    let max_inner = LUA_IDSIZE.saturating_sub(b"[string \"...\"]".len());
    let mut chunk_id = b"[string \"".to_vec();
    if needs_ellipsis || before_newline.len() > max_inner {
        chunk_id.extend_from_slice(&before_newline[..before_newline.len().min(max_inner)]);
        chunk_id.extend_from_slice(b"...\"]");
    } else {
        chunk_id.extend_from_slice(before_newline);
        chunk_id.extend_from_slice(b"\"]");
    }
    chunk_id
}

fn function_name_in_env(l: &LuaState, func_ref: GcRef<Function>) -> Option<Vec<u8>> {
    // SAFETY: the function is held by the active stack frame or an argument.
    let env = unsafe { func_ref.as_ref() }
        .and_then(|function| function.env())
        .or(l.global_table)?;
    // SAFETY: env is reachable from the function/global state while getinfo runs.
    let table = unsafe { env.as_ref() }?;
    for (key, value) in table.hash_entries() {
        if let (Value::String(key_ref), Value::Function(value_func)) = (key, value)
            && *value_func == func_ref
            && let Ok(key_string) = l.copy_string_bytes(*key_ref)
        {
            return Some(key_string);
        }
    }
    None
}

fn function_name_in_global(l: &LuaState, func_ref: GcRef<Function>) -> Option<Vec<u8>> {
    let env = l.global_table?;
    // SAFETY: global table is rooted by the Lua state while debugging.
    let table = unsafe { env.as_ref() }?;
    for (key, value) in table.hash_entries() {
        if let (Value::String(key_ref), Value::Function(value_func)) = (key, value)
            && *value_func == func_ref
            && let Ok(key_string) = l.copy_string_bytes(*key_ref)
        {
            return Some(key_string);
        }
    }
    None
}

fn publish_debug_info(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    info: DebugInfo,
) -> Result<(), GcRefValidationError> {
    gc.with_publication(|transaction| {
        let table = transaction.alloc(Table::new());
        set_string_field(state, transaction, &table, b"source", &info.source)?;
        set_string_field(state, transaction, &table, b"short_src", &info.short_src)?;
        set_string_field(state, transaction, &table, b"what", &info.what)?;
        if let Some(line) = info.currentline {
            set_number_field(state, transaction, &table, b"currentline", line as f64)?;
        }
        if let Some(line) = info.linedefined {
            set_number_field(state, transaction, &table, b"linedefined", line as f64)?;
        }
        if let Some(line) = info.lastlinedefined {
            set_number_field(state, transaction, &table, b"lastlinedefined", line as f64)?;
        }
        if let Some(name) = &info.name {
            set_string_field(state, transaction, &table, b"name", name)?;
        }
        set_string_field(state, transaction, &table, b"namewhat", &info.namewhat)?;
        if let Some(function) = info.func {
            set_value_field(state, transaction, &table, b"func", &function)?;
        }
        if let Some(nups) = info.nups {
            set_number_field(state, transaction, &table, b"nups", nups as f64)?;
        }
        if !info.active_lines.is_empty() {
            let key = crate::registration::rooted_bytes(state, transaction, b"activelines")?;
            let active_lines = active_lines_table(transaction, &info.active_lines)?;
            transaction.set_table_table(&table, &key, &active_lines)?;
        }

        // SAFETY: the completed info Table is installed on the active stack.
        unsafe { transaction.publish_table_value(table, |value| state.push_value(value)) }
    })
}

fn set_string_field<'scope>(
    state: &LuaState,
    transaction: &mut PublicationTxn<'scope>,
    table: &Rooted<'scope, Table>,
    key: &[u8],
    value: &[u8],
) -> Result<(), GcRefValidationError> {
    let key = crate::registration::rooted_bytes(state, transaction, key)?;
    let value = crate::registration::rooted_bytes(state, transaction, value)?;
    transaction.set_table_string(table, &key, &value)
}

fn set_number_field<'scope>(
    state: &LuaState,
    transaction: &mut PublicationTxn<'scope>,
    table: &Rooted<'scope, Table>,
    key: &[u8],
    value: f64,
) -> Result<(), GcRefValidationError> {
    let key = crate::registration::rooted_bytes(state, transaction, key)?;
    transaction.set_table_value(table, &key, &Value::Number(value))
}

fn set_value_field<'scope>(
    state: &LuaState,
    transaction: &mut PublicationTxn<'scope>,
    table: &Rooted<'scope, Table>,
    key: &[u8],
    value: &Value,
) -> Result<(), GcRefValidationError> {
    let key = crate::registration::rooted_bytes(state, transaction, key)?;
    transaction.set_table_value(table, &key, value)
}

fn active_lines_table<'scope>(
    transaction: &mut PublicationTxn<'scope>,
    lines: &[i32],
) -> Result<Rooted<'scope, Table>, GcRefValidationError> {
    let table = transaction.alloc(Table::new());
    for line in lines {
        transaction.set_table_entry(&table, &Value::Number(*line as f64), &Value::Boolean(true))?;
    }
    Ok(table)
}

fn upvalue_name(gc: &GarbageCollector, func_ref: GcRef<Function>, index: usize) -> Option<Vec<u8>> {
    let proto_ref = gc.with_ref(func_ref, Function::proto).ok().flatten()?;
    let proto_ptr = gc.validate_ref(proto_ref).ok()?;
    // SAFETY: validated above; destructive sweep is disabled throughout this
    // debug C callback.
    let proto = unsafe { proto_ptr.as_ref() };
    let name_ref = proto.upvalue_name(index)?;
    gc.with_string_bytes(name_ref, <[u8]>::to_vec).ok()
}

fn function_upvalue(
    func_ref: GcRef<Function>,
    index: usize,
) -> Option<GcRef<lua_core::upvalue::Upvalue>> {
    // SAFETY: function argument is on the active Lua stack.
    let func = unsafe { func_ref.as_ref() }?;
    func.upvalue(index)
}

fn get_upvalue_value(
    l: &LuaState,
    gc: &GarbageCollector,
    upvalue_ref: GcRef<Upvalue>,
) -> Result<Value, &'static str> {
    let (location, closed) = gc
        .with_ref(upvalue_ref, |upvalue| {
            (
                upvalue.open_location(),
                upvalue
                    .is_closed()
                    .then(|| upvalue.get_closed_value().clone()),
            )
        })
        .map_err(|_| "debug.getupvalue received an invalid Upvalue")?;
    if let Some((owner, stack_index)) = location {
        if l.state_handle() != Some(owner) {
            return Err("debug.getupvalue cross-state open Upvalue access is not yet scheduled");
        }
        if !l.open_upvalues.contains(&upvalue_ref) {
            return Err("debug.getupvalue owner state lost its open Upvalue");
        }
        l.stack
            .at(stack_index)
            .cloned()
            .ok_or("debug.getupvalue open stack index is out of range")
    } else {
        Ok(closed.unwrap_or(Value::Nil))
    }
}

fn set_upvalue_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    upvalue_ref: GcRef<Upvalue>,
    value: Value,
) -> Result<(), &'static str> {
    let location = gc
        .with_ref(upvalue_ref, Upvalue::open_location)
        .map_err(|_| "debug.setupvalue received an invalid Upvalue")?;
    if let Some((owner, stack_index)) = location {
        if l.state_handle() != Some(owner) {
            return Err("debug.setupvalue cross-state open Upvalue access is not yet scheduled");
        }
        if !l.open_upvalues.contains(&upvalue_ref) {
            return Err("debug.setupvalue owner state lost its open Upvalue");
        }
        let slot = l
            .stack
            .at_mut(stack_index)
            .ok_or("debug.setupvalue open stack index is out of range")?;
        *slot = value;
    } else {
        gc.with_mut(upvalue_ref, |upvalue| upvalue.set_closed_value(value))
            .map_err(|_| "debug.setupvalue received an invalid Upvalue")?;
    }
    Ok(())
}

fn function_env(func_ref: GcRef<Function>) -> Option<GcRef<Table>> {
    // SAFETY: function refs are held by a Lua stack or GC object.
    unsafe { func_ref.as_ref() }.and_then(|function| function.env())
}

fn set_function_env(func_ref: GcRef<Function>, env: GcRef<Table>) {
    // SAFETY: function refs are held by a Lua stack or GC object.
    unsafe { &mut *(func_ref.as_ptr() as *mut Function) }.set_env(Some(env));
}

fn thread_env(current: &mut LuaState, thread_ref: GcRef<Thread>) -> Option<GcRef<Table>> {
    with_thread_state_mut(current, thread_ref, |state| {
        state.thread_env.or(state.global_table)
    })
    .flatten()
}

fn with_thread_state_mut<T>(
    current: &mut LuaState,
    thread_ref: GcRef<Thread>,
    f: impl for<'state> FnOnce(&'state mut LuaState) -> T,
) -> Option<T> {
    if thread_ref.is_null() {
        return None;
    }
    // SAFETY: thread_ref is held by a Lua stack or GC object.
    let thread = unsafe { thread_ref.as_ref() }?;
    let handle = thread.state_handle()?;
    current.with_resolved_state_mut(handle, f).ok()
}

#[cfg(test)]
mod byte_string_tests {
    use super::*;
    use lua_core::string_pool::StringPool;

    unsafe extern "C" fn test_hook(_state: *mut std::ffi::c_void) -> i32 {
        0
    }

    fn string_bytes(state: &LuaState, value: &Value) -> Vec<u8> {
        let Value::String(string_ref) = value else {
            panic!("expected Lua string, got {value:?}");
        };
        state
            .copy_string_bytes(*string_ref)
            .expect("test string is live")
    }

    #[test]
    fn options_are_matched_as_exact_lua_bytes() {
        assert!(name_only_options(Some(b"n")));
        assert!(name_only_options(Some(b"nnn")));
        assert!(!name_only_options(Some(b"")));
        assert!(!name_only_options(Some(b"n\0n")));
        assert!(!name_only_options(Some(&[b'n', 0xff])));
    }

    #[test]
    fn source_and_short_source_preserve_nul_and_invalid_utf8() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let source_bytes = b"=\xff\0chunk";
        let source_ref = string_pool.intern_bytes(&mut gc, source_bytes);
        let mut proto = Proto::new();
        proto.set_source(Some(source_ref));
        let proto_ref = gc.create(proto);
        let function_ref = gc.create(Function::new_lua(proto_ref));

        let info = function_info(&gc, function_ref).expect("Lua function has debug info");
        assert_eq!(info.source, source_bytes);
        assert_eq!(info.short_src, b"\xff\0chunk");

        let raw_chunk = [0xff, 0, 0x80, b'x'];
        let mut expected = b"[string \"".to_vec();
        expected.extend_from_slice(&raw_chunk);
        expected.extend_from_slice(b"\"]");
        assert_eq!(short_source(&raw_chunk), expected);
    }

    #[test]
    fn traceback_preserves_message_bytes_before_ascii_suffix() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut state = LuaState::new();
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut string_pool);
        let message = [0xff, 0, 0x80, b'x'];
        let message_ref = string_pool.intern_bytes(&mut gc, &message);
        state.push_value(Value::String(message_ref));

        // SAFETY: the callback receives a valid LuaState pointer for exactly
        // this dynamic call, matching the VM C-function ABI.
        let results = unsafe {
            lua_debug_traceback((&mut state as *mut LuaState).cast::<std::ffi::c_void>())
        };
        assert_eq!(results, 1);

        let output = state.at(-1).expect("traceback result is on the stack");
        let mut expected = message.to_vec();
        expected.extend_from_slice(b"\nstack traceback:\n");
        assert_eq!(string_bytes(&state, output), expected);
    }

    #[test]
    fn hook_mask_round_trips_every_byte_including_nul() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut state = LuaState::new();
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut string_pool);
        let hook_ref = gc.create(Function::new_c(test_hook));
        let mask: Vec<u8> = (0..=u8::MAX).collect();
        let mask_ref = string_pool.intern_bytes(&mut gc, &mask);
        state.push_value(Value::Function(hook_ref));
        state.push_value(Value::String(mask_ref));
        state.push_number(7.0);

        assert_eq!(
            // SAFETY: the callback receives the live test state only for this
            // dynamic ABI call.
            unsafe { lua_debug_sethook((&mut state as *mut LuaState).cast()) },
            0
        );
        state.set_top(0);
        assert_eq!(
            // SAFETY: the callback receives the live test state only for this
            // dynamic ABI call.
            unsafe { lua_debug_gethook((&mut state as *mut LuaState).cast()) },
            3
        );
        assert_eq!(
            string_bytes(&state, state.at(2).expect("gethook returns mask second")),
            mask
        );
        assert_eq!(mask_storage_to_bytes(&state.debug_hook_mask), mask);
    }
}

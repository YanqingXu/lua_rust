//! Minimal coroutine library.

use lua_core::function::RuntimeNativeFunction;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::thread::{CoroutineStatus, Thread};
use lua_core::value::Value;
use lua_vm::runtime::PendingStateError;
use lua_vm::state::{LuaState, ThreadStatus};

pub fn open_coroutine(l: &mut LuaState, gc: &mut GarbageCollector) {
    let coroutine_table = find_lib_table(l, "coroutine");
    if coroutine_table.is_null() {
        return;
    }

    reg(l, gc, coroutine_table, "create", lua_coroutine_create);
    reg_runtime_native(
        l,
        gc,
        coroutine_table,
        "resume",
        RuntimeNativeFunction::CoroutineResume,
    );
    reg(l, gc, coroutine_table, "running", lua_coroutine_running);
    reg(l, gc, coroutine_table, "status", lua_coroutine_status);
    reg(l, gc, coroutine_table, "wrap", lua_coroutine_wrap);
    reg(l, gc, coroutine_table, "yield", lua_coroutine_yield);
}

fn reg(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), func, None)
        .expect("coroutine Function publication must remain collector-valid");
}

fn reg_runtime_native(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    operation: RuntimeNativeFunction,
) {
    crate::registration::register_runtime_native(state, gc, table, name.as_bytes(), operation)
        .expect("coroutine Runtime-native publication must remain collector-valid");
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
                && key_str.as_bytes() == name.as_bytes()
                && let Value::Table(t) = val
            {
                return *t;
            }
        }
    }
    GcRef::null()
}

unsafe extern "C" fn lua_coroutine_create(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let func = l.at(1).cloned().unwrap_or(Value::Nil);
    if !matches!(func, Value::Function(_)) {
        return push_error(l, b"bad argument #1 to 'create' (function expected)");
    }

    match publish_thread_to_stack(l, gc, func) {
        Ok(()) => 1,
        Err(error) => {
            push_lua_bytes(l, gc, error.as_bytes());
            -1
        }
    }
}

unsafe extern "C" fn lua_coroutine_status(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let Value::Thread(thread_ref) = l.at(1).cloned().unwrap_or(Value::Nil) else {
        l.push_nil();
        return 1;
    };
    let status = with_thread(thread_ref, |thread| thread.status())
        .map(status_name)
        .unwrap_or(b"dead");
    push_lua_bytes(l, gc, status);
    1
}

unsafe extern "C" fn lua_coroutine_running(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    if let Some(thread_ref) = l.current_thread {
        l.push_value(Value::Thread(thread_ref));
    } else {
        l.push_nil();
    }
    1
}

unsafe extern "C" fn lua_coroutine_wrap(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let func = l.at(1).cloned().unwrap_or(Value::Nil);
    if !matches!(func, Value::Function(_)) {
        return push_error(l, b"bad argument #1 to 'wrap' (function expected)");
    }

    match publish_wrapper_to_stack(l, gc, func) {
        Ok(()) => 1,
        Err(error) => {
            push_lua_bytes(l, gc, error.as_bytes());
            -1
        }
    }
}

unsafe extern "C" fn lua_coroutine_yield(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    l.yielded_values = args_from(l, 1);
    l.status = ThreadStatus::Yield;
    0
}

fn coroutine_state(l: &LuaState, gc: &mut GarbageCollector, entry: Value) -> LuaState {
    let mut state = if let Some(global) = l.global_table {
        LuaState::with_global_table(global)
    } else {
        LuaState::new()
    };
    state.string_pool = l.string_pool;
    state.gc = Some(gc as *mut GarbageCollector);
    state.thread_env = l.thread_env.or(l.global_table);
    state.chunk_env = l.chunk_env.or(l.thread_env).or(l.global_table);
    state.nil_metatable = l.nil_metatable;
    state.boolean_metatable = l.boolean_metatable;
    state.number_metatable = l.number_metatable;
    state.push_value(entry);
    state
}

fn publish_thread_to_stack(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    entry: Value,
) -> Result<(), String> {
    let state = coroutine_state(l, gc, entry);
    let published: Result<(), PendingStateError> = gc.with_publication(|transaction| {
        let thread = transaction.alloc(Thread::new());
        l.with_pending_coroutine_state(state, |pending, publisher| {
            pending.bind_thread(transaction, thread)?;
            pending.publish_thread_to_stack(publisher)?;
            Ok(())
        })?
    });
    published.map_err(|error| format!("invalid coroutine publication: {error}"))
}

fn publish_wrapper_to_stack(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    entry: Value,
) -> Result<(), String> {
    let state = coroutine_state(l, gc, entry);
    let published: Result<(), PendingStateError> = gc.with_publication(|transaction| {
        let thread = transaction.alloc(Thread::new());
        l.with_pending_coroutine_state(state, |pending, publisher| {
            let upvalue = transaction.alloc_closed_thread_upvalue(&thread)?;
            let wrapper = transaction.alloc_runtime_native_with_upvalue(
                RuntimeNativeFunction::CoroutineWrapRunner,
                &upvalue,
            )?;
            pending.bind_thread(transaction, thread)?;
            pending.publish_wrapper_to_stack(transaction, wrapper, publisher)?;
            Ok(())
        })?
    });
    published.map_err(|error| format!("invalid coroutine publication: {error}"))
}

fn args_from(l: &LuaState, first: i32) -> Vec<Value> {
    let top = l.get_top();
    if top < first {
        return Vec::new();
    }
    (first..=top)
        .map(|idx| l.at(idx).cloned().unwrap_or(Value::Nil))
        .collect()
}

fn with_thread<T>(
    thread_ref: GcRef<Thread>,
    f: impl for<'thread> FnOnce(&'thread Thread) -> T,
) -> Option<T> {
    // SAFETY: the thread value is held on a Lua stack or in a closure upvalue.
    let thread = unsafe { thread_ref.as_ref() }?;
    Some(f(thread))
}

fn status_name(status: CoroutineStatus) -> &'static [u8] {
    match status {
        CoroutineStatus::Suspended => b"suspended",
        CoroutineStatus::Running => b"running",
        CoroutineStatus::Normal => b"normal",
        CoroutineStatus::Dead => b"dead",
    }
}

fn push_lua_bytes(l: &mut LuaState, gc: &mut GarbageCollector, bytes: &[u8]) {
    let _ = crate::registration::push_string(l, gc, bytes);
}

fn push_error(l: &mut LuaState, message: &'static [u8]) -> i32 {
    if let Some(gc_ptr) = l.gc {
        // SAFETY: LuaState::gc is installed by the VM before calling C functions.
        let gc = unsafe { &mut *gc_ptr };
        push_lua_bytes(l, gc, message);
    } else {
        l.push_nil();
    }
    -1
}

#[cfg(test)]
mod byte_string_tests {
    use super::*;
    use lua_core::gc_string::GcString;

    #[test]
    fn library_lookup_requires_the_exact_ascii_key_bytes() {
        let mut gc = GarbageCollector::new();
        let target = gc.create(Table::new());
        let decoy = gc.create(Table::new());
        let exact_key = gc.create(GcString::from_bytes(b"coroutine"));
        let invalid_prefix_key = gc.create(GcString::from_bytes(b"coroutine\0\xff"));
        let mut global = Table::new();
        global.set(&Value::String(invalid_prefix_key), &Value::Table(decoy));
        global.set(&Value::String(exact_key), &Value::Table(target));
        let global_ref = gc.create(global);
        let state = LuaState::with_global_table(global_ref);

        assert_eq!(find_lib_table(&state, "coroutine"), target);
        assert_ne!(find_lib_table(&state, "coroutine"), decoy);
    }

    #[test]
    fn status_protocol_values_are_ascii_bytes() {
        assert_eq!(status_name(CoroutineStatus::Suspended), b"suspended");
        assert_eq!(status_name(CoroutineStatus::Running), b"running");
        assert_eq!(status_name(CoroutineStatus::Normal), b"normal");
        assert_eq!(status_name(CoroutineStatus::Dead), b"dead");
    }

    #[test]
    fn lua_byte_results_preserve_nul_and_invalid_utf8() {
        let mut gc = GarbageCollector::new();
        let mut state = LuaState::new();
        let bytes = [0, 0xff, 0x80, b'x'];

        push_lua_bytes(&mut state, &mut gc, &bytes);
        let Value::String(string_ref) = state.at(-1).expect("result is pushed") else {
            panic!("expected Lua string result");
        };
        // SAFETY: the collector remains alive and no collection runs.
        let string = unsafe { string_ref.as_ref() }.expect("test string is live");
        assert_eq!(string.as_bytes(), bytes);
    }
}

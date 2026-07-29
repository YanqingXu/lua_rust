//! Minimal coroutine library.

use lua_core::function::{Function, RuntimeNativeFunction};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::thread::{CoroutineStatus, Thread};
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use lua_vm::state::{LuaState, ThreadStatus};

pub fn open_coroutine(l: &mut LuaState, gc: &mut GarbageCollector) {
    let coroutine_table = find_lib_table(l, "coroutine");
    if coroutine_table.is_null() {
        return;
    }

    let table_ptr = coroutine_table.as_ptr() as *mut Table;
    reg(gc, table_ptr, "create", lua_coroutine_create);
    reg_runtime_native(
        gc,
        table_ptr,
        "resume",
        RuntimeNativeFunction::CoroutineResume,
    );
    reg(gc, table_ptr, "running", lua_coroutine_running);
    reg(gc, table_ptr, "status", lua_coroutine_status);
    reg(gc, table_ptr, "wrap", lua_coroutine_wrap);
    reg(gc, table_ptr, "yield", lua_coroutine_yield);
}

fn reg(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::from_bytes(name.as_bytes()));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table points to the library table created and rooted by open_library.
    unsafe {
        (*table).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

fn reg_runtime_native(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    operation: RuntimeNativeFunction,
) {
    let name_str = gc.create(GcString::from_bytes(name.as_bytes()));
    let func_obj = gc.create(Function::new_runtime_native(operation));
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

    match create_thread(l, gc, func) {
        Ok(thread_ref) => {
            l.push_value(Value::Thread(thread_ref));
            1
        }
        Err(error) => {
            l.push_value(error);
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

    let thread_ref = match create_thread(l, gc, func) {
        Ok(thread_ref) => thread_ref,
        Err(error) => {
            l.push_value(error);
            return -1;
        }
    };
    let upvalue_ref = gc.create(Upvalue::new_closed(Value::Thread(thread_ref)));
    let mut wrapper = Function::new_runtime_native(RuntimeNativeFunction::CoroutineWrapRunner);
    wrapper.add_upvalue(upvalue_ref);
    let wrapper_ref = gc.create(wrapper);
    l.push_value(Value::Function(wrapper_ref));
    1
}

unsafe extern "C" fn lua_coroutine_yield(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    l.yielded_values = args_from(l, 1);
    l.status = ThreadStatus::Yield;
    0
}

fn create_thread(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    entry: Value,
) -> Result<GcRef<Thread>, Value> {
    let mut co_state = if let Some(global) = l.global_table {
        LuaState::with_global_table(global)
    } else {
        LuaState::new()
    };
    co_state.string_pool = l.string_pool;
    co_state.gc = Some(gc as *mut GarbageCollector);
    co_state.thread_env = l.thread_env.or(l.global_table);
    co_state.chunk_env = l.chunk_env.or(l.thread_env).or(l.global_table);
    co_state.nil_metatable = l.nil_metatable;
    co_state.boolean_metatable = l.boolean_metatable;
    co_state.number_metatable = l.number_metatable;
    co_state.push_value(entry);

    // Publish the owning Thread into the not-yet-inserted state before the
    // arena makes that state addressable. This avoids borrowing a second
    // LuaState merely to complete the State -> Thread edge after insertion.
    // The reverse Thread -> StateHandle edge is installed before the Thread
    // value is exposed to Lua. A future PendingState transaction will make
    // allocation failure rollback and temporary-root tracing explicit.
    let thread_ref = gc.create(Thread::new());
    co_state.current_thread = Some(thread_ref);
    let handle = l
        .insert_coroutine_state(co_state)
        .map_err(|err| diagnostic_string_value(gc, &format!("invalid coroutine state: {err}")))?;
    with_thread_mut(thread_ref, |thread| thread.set_state_handle(handle))
        .ok_or_else(|| lua_ascii_value(gc, b"invalid coroutine"))?;
    Ok(thread_ref)
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

fn with_thread_mut<T>(
    thread_ref: GcRef<Thread>,
    f: impl for<'thread> FnOnce(&'thread mut Thread) -> T,
) -> Option<T> {
    if thread_ref.is_null() {
        return None;
    }
    // SAFETY: the thread is held by a live Lua value and coroutine mutations
    // are serialized by the single-threaded VM. The HRTB closure prevents the
    // mutable reference from escaping this call.
    Some(f(unsafe { &mut *(thread_ref.as_ptr() as *mut Thread) }))
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
    let s = gc.create(GcString::from_bytes(bytes));
    l.push_value(Value::String(s));
}

fn lua_ascii_value(gc: &mut GarbageCollector, bytes: &'static [u8]) -> Value {
    debug_assert!(bytes.is_ascii());
    Value::String(gc.create(GcString::from_bytes(bytes)))
}

/// Convert host diagnostic text at the explicit UTF-8 boundary.
fn diagnostic_string_value(gc: &mut GarbageCollector, text: &str) -> Value {
    Value::String(gc.create(GcString::from_utf8_text(text)))
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

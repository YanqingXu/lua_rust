//! 表库 (Table Library)
#![allow(clippy::not_unsafe_ptr_arg_deref)]
//!

use lua_core::function::RuntimeNativeFunction;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

pub fn open_table(l: &mut LuaState, gc: &mut GarbageCollector) {
    let table_lib = find_lib_table(l, gc, "table");
    if table_lib.is_null() {
        return;
    }

    reg(l, gc, table_lib, "concat", lua_table_concat);
    crate::registration::register_runtime_native(
        l,
        gc,
        table_lib,
        b"foreach",
        RuntimeNativeFunction::TableForeach,
    )
    .expect("table.foreach Runtime-native publication must remain collector-valid");
    crate::registration::register_runtime_native(
        l,
        gc,
        table_lib,
        b"foreachi",
        RuntimeNativeFunction::TableForeachi,
    )
    .expect("table.foreachi Runtime-native publication must remain collector-valid");
    reg(l, gc, table_lib, "getn", lua_table_getn);
    reg(l, gc, table_lib, "insert", lua_table_insert);
    reg(l, gc, table_lib, "maxn", lua_table_maxn);
    reg(l, gc, table_lib, "remove", lua_table_remove);
    crate::registration::register_runtime_native(
        l,
        gc,
        table_lib,
        b"sort",
        RuntimeNativeFunction::TableSort,
    )
    .expect("table.sort Runtime-native publication must remain collector-valid");
}

fn reg(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), func, None)
        .expect("table Function publication must remain collector-valid");
}

fn find_lib_table(l: &LuaState, gc: &GarbageCollector, name: &str) -> GcRef<Table> {
    crate::registration::find_library_table_with_gc(l, gc, name.as_bytes())
        .ok()
        .flatten()
        .unwrap_or_else(GcRef::null)
}

fn table_arg(l: &LuaState, idx: i32) -> Option<GcRef<Table>> {
    match l.at(idx) {
        Some(Value::Table(t)) => Some(*t),
        _ => None,
    }
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

fn string_arg_bytes(l: &LuaState, idx: i32) -> Option<Vec<u8>> {
    match l.at(idx) {
        Some(Value::String(s)) => l.copy_string_bytes(*s).ok(),
        Some(Value::Number(n)) => Some(number_to_lua_string(*n).into_bytes()),
        Some(Value::Nil) | None => None,
        _ => None,
    }
}

fn number_to_lua_string(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

fn value_to_concat_bytes(l: &LuaState, value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::String(s) => l.copy_string_bytes(*s).ok(),
        Value::Number(n) => Some(number_to_lua_string(*n).into_bytes()),
        _ => None,
    }
}

fn append_concat_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ()> {
    output.len().checked_add(bytes.len()).ok_or(())?;
    output.try_reserve(bytes.len()).map_err(|_| ())?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn push_lua_bytes(l: &mut LuaState, bytes: &[u8]) -> bool {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    crate::registration::push_string(l, gc, bytes).is_ok()
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    push_lua_bytes(l, text.as_bytes())
}

fn table_error(l: &mut LuaState, message: &str) -> i32 {
    let _ = push_lua_string(l, message);
    -1
}

unsafe extern "C" fn lua_table_getn(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    let Ok(length) = l.with_gc_ref(table_ref, Table::length) else {
        return -1;
    };
    l.push_value(Value::Number(length as f64));
    1
}

unsafe extern "C" fn lua_table_maxn(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    let Ok(max_index) = l.with_gc_ref(table_ref, |table| {
        let mut max_index: f64 = 0.0;
        for i in 1..=table.array_size() {
            if !table.get_array(i as i32).is_nil() {
                max_index = max_index.max(i as f64);
            }
        }
        for (key, value) in table.hash_entries() {
            if !value.is_nil()
                && let Value::Number(n) = key
                && n.is_finite()
                && *n > max_index
            {
                max_index = *n;
            }
        }
        max_index
    }) else {
        return -1;
    };

    l.push_value(Value::Number(max_index));
    1
}

unsafe extern "C" fn lua_table_insert(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    if nargs < 2 {
        return -1;
    }

    let Some(gc_ptr) = l.active_gc_ptr() else {
        return -1;
    };
    // SAFETY: the VM installs this scoped service pointer for the callback.
    let gc = unsafe { &mut *gc_ptr };
    let Ok(len) = gc.with_ref(table_ref, |table| table.length() as i32) else {
        return -1;
    };
    if nargs == 2 {
        let value = l.at(2).cloned().unwrap_or(Value::Nil);
        if gc
            .with_mut(table_ref, |table| {
                table.set(&Value::Number((len + 1) as f64), &value);
            })
            .is_err()
        {
            return -1;
        }
        return 0;
    }

    let Some(pos_num) = number_arg(l, 2) else {
        return -1;
    };
    let pos = pos_num as i32;
    let value = l.at(3).cloned().unwrap_or(Value::Nil);
    if gc
        .with_mut(table_ref, |table| {
            for i in (pos..=len).rev() {
                let shifted = table.get(&Value::Number(i as f64));
                table.set(&Value::Number((i + 1) as f64), &shifted);
            }
            table.set(&Value::Number(pos as f64), &value);
        })
        .is_err()
    {
        return -1;
    }
    0
}

unsafe extern "C" fn lua_table_remove(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return -1;
    };
    // SAFETY: the VM installs this scoped service pointer for the callback.
    let gc = unsafe { &mut *gc_ptr };
    let Ok(len) = gc.with_ref(table_ref, |table| table.length() as i32) else {
        return -1;
    };
    let pos = if nargs >= 2 {
        number_arg(l, 2).unwrap_or(0.0) as i32
    } else {
        len
    };

    if pos < 1 || pos > len {
        l.push_value(Value::Nil);
        return 1;
    }

    let Ok(removed) = gc.with_mut(table_ref, |table| {
        let removed = table.get(&Value::Number(pos as f64));
        for i in pos..len {
            let shifted = table.get(&Value::Number((i + 1) as f64));
            table.set(&Value::Number(i as f64), &shifted);
        }
        table.remove(&Value::Number(len as f64));
        removed
    }) else {
        return -1;
    };
    l.push_value(removed);
    1
}

unsafe extern "C" fn lua_table_concat(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    let sep = if nargs >= 2 {
        string_arg_bytes(l, 2).unwrap_or_default()
    } else {
        Vec::new()
    };
    let start = if nargs >= 3 {
        number_arg(l, 3).unwrap_or(1.0) as i32
    } else {
        1
    };
    let requested_end = (nargs >= 4).then(|| number_arg(l, 4)).flatten();
    let values = l.with_gc_ref(table_ref, |table| {
        let end = if nargs >= 4 {
            requested_end.unwrap_or(table.length() as f64) as i32
        } else {
            table.length() as i32
        };
        (start..=end)
            .map(|idx| table.get(&Value::Number(idx as f64)))
            .collect::<Vec<_>>()
    });
    let Ok(values) = values else {
        return -1;
    };

    let mut result = Vec::new();
    for (offset, value) in values.into_iter().enumerate() {
        if offset > 0 && append_concat_bytes(&mut result, &sep).is_err() {
            return table_error(l, "table.concat: result length overflow");
        }
        let Some(piece) = value_to_concat_bytes(l, &value) else {
            return -1;
        };
        if append_concat_bytes(&mut result, &piece).is_err() {
            return table_error(l, "table.concat: result length overflow");
        }
    }
    if push_lua_bytes(l, &result) { 1 } else { -1 }
}

#[cfg(test)]
mod byte_string_tests {
    use super::*;
    use lua_core::string_pool::StringPool;

    #[test]
    fn concat_value_preserves_nul_and_invalid_utf8() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let value = Value::String(string_pool.intern_bytes(&mut gc, &[0, 0x80, 0xff]));
        let mut state = LuaState::new();

        lua_vm::with_vm_context(&mut state, &mut gc, &mut string_pool, |state| {
            assert_eq!(
                value_to_concat_bytes(state, &value),
                Some(vec![0x00, 0x80, 0xff])
            );
        });
    }

    #[test]
    fn default_string_order_uses_vm_nul_segment_rule() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut state = LuaState::new();
        let left = Value::String(string_pool.intern_bytes(&mut gc, b"a\0b"));
        let right = Value::String(string_pool.intern_bytes(&mut gc, b"a\0c"));

        lua_vm::with_vm_context_gc(&mut state, &mut gc, &mut string_pool, |_, gc| {
            let ordering = gc
                .with_string_bytes(left.try_as_string().expect("left string"), |left| {
                    gc.with_string_bytes(right.try_as_string().expect("right string"), |right| {
                        lua_vm::execute::compare_lua_string_bytes(left, right)
                    })
                })
                .and_then(|ordering| ordering)
                .expect("both strings remain valid");
            assert_eq!(ordering, std::cmp::Ordering::Less);
        });
    }
}

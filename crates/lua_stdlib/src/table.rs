//! 表库 (Table Library)
#![allow(clippy::not_unsafe_ptr_arg_deref)]
//!

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::RuntimeError;
use lua_vm::execute::{call_value_with_results, compare_lua_string_bytes};
use lua_vm::state::LuaState;
use std::cmp::Ordering;

pub fn open_table(l: &mut LuaState, gc: &mut GarbageCollector) {
    let table_lib = find_lib_table(l, "table");
    if table_lib.is_null() {
        return;
    }

    reg(l, gc, table_lib, "concat", lua_table_concat);
    reg(l, gc, table_lib, "foreach", lua_table_foreach);
    reg(l, gc, table_lib, "foreachi", lua_table_foreachi);
    reg(l, gc, table_lib, "getn", lua_table_getn);
    reg(l, gc, table_lib, "insert", lua_table_insert);
    reg(l, gc, table_lib, "maxn", lua_table_maxn);
    reg(l, gc, table_lib, "remove", lua_table_remove);
    reg(l, gc, table_lib, "sort", lua_table_sort);
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

fn find_lib_table(l: &LuaState, name: &str) -> GcRef<Table> {
    crate::registration::find_library_table(l, name.as_bytes())
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

#[derive(Debug)]
enum TableSortError {
    Message(String),
    ErrorValue(Value),
}

impl TableSortError {
    fn from_runtime(err: RuntimeError) -> Self {
        err.error_value()
            .map(TableSortError::ErrorValue)
            .unwrap_or(TableSortError::Message(err.message))
    }
}

fn table_error(l: &mut LuaState, message: &str) -> i32 {
    let _ = push_lua_string(l, message);
    -1
}

fn push_sort_error(l: &mut LuaState, err: TableSortError) -> i32 {
    match err {
        TableSortError::Message(message) => {
            let _ = push_lua_string(l, &message);
        }
        TableSortError::ErrorValue(value) => l.push_value(value),
    }
    -1
}

fn is_truthy(value: &Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}

fn table_foreachi_limit(l: &LuaState, table: &Table) -> usize {
    if let Ok(Some(key)) = l.find_interned_string(b"n")
        && let Value::Number(n) = table.get(&Value::String(key))
        && n >= 0.0
    {
        return n as usize;
    }
    table.length()
}

fn value_metatable(gc: &GarbageCollector, value: &Value) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table_ref) => gc.with_ref(*table_ref, Table::metatable).ok().flatten(),
        Value::Userdata(userdata_ref) => gc
            .with_ref(*userdata_ref, |userdata| userdata.metatable())
            .ok()
            .flatten(),
        _ => None,
    }
}

fn lookup_metamethod(l: &LuaState, metatable: GcRef<Table>, name: &str) -> Option<Value> {
    crate::registration::find_table_field(l, metatable, name.as_bytes())
        .ok()
        .flatten()
}

fn find_less_metamethod(
    l: &LuaState,
    gc: &GarbageCollector,
    lhs: &Value,
    rhs: &Value,
) -> Option<Value> {
    value_metatable(gc, lhs)
        .and_then(|mt| lookup_metamethod(l, mt, "__lt"))
        .or_else(|| value_metatable(gc, rhs).and_then(|mt| lookup_metamethod(l, mt, "__lt")))
}

fn compare_with_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    comparator: &Value,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, TableSortError> {
    let mut comparison = false;
    call_value_with_results(
        l,
        gc,
        comparator.clone(),
        &[lhs.clone(), rhs.clone()],
        Some(1),
        |_, _, results| {
            comparison = results.first().is_some_and(is_truthy);
        },
    )
    .map_err(TableSortError::from_runtime)?;
    Ok(comparison)
}

fn default_less(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, TableSortError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a < b),
        (Value::String(lhs), Value::String(rhs)) => gc
            .with_string_bytes(*lhs, |lhs| {
                gc.with_string_bytes(*rhs, |rhs| compare_lua_string_bytes(lhs, rhs))
            })
            .and_then(|ordering| ordering)
            .map(|ordering| ordering == Ordering::Less)
            .map_err(|error| TableSortError::Message(format!("invalid string key: {error}"))),
        _ => {
            if let Some(metamethod) = find_less_metamethod(l, gc, lhs, rhs) {
                return compare_with_function(l, gc, &metamethod, lhs, rhs);
            }
            Err(TableSortError::Message(
                "attempt to compare values in table.sort".to_string(),
            ))
        }
    }
}

fn sort_less(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    comparator: &Option<Value>,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, TableSortError> {
    if let Some(comparator) = comparator {
        compare_with_function(l, gc, comparator, lhs, rhs)
    } else {
        default_less(l, gc, lhs, rhs)
    }
}

fn merge_sort_values(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    values: &mut [Value],
    comparator: &Option<Value>,
) -> Result<(), TableSortError> {
    let len = values.len();
    if len <= 1 {
        return Ok(());
    }

    let mid = len / 2;
    merge_sort_values(l, gc, &mut values[..mid], comparator)?;
    merge_sort_values(l, gc, &mut values[mid..], comparator)?;

    let left = values[..mid].to_vec();
    let right = values[mid..].to_vec();
    let mut left_idx = 0;
    let mut right_idx = 0;
    let mut out_idx = 0;

    while left_idx < left.len() && right_idx < right.len() {
        if sort_less(l, gc, comparator, &right[right_idx], &left[left_idx])? {
            values[out_idx] = right[right_idx].clone();
            right_idx += 1;
        } else {
            values[out_idx] = left[left_idx].clone();
            left_idx += 1;
        }
        out_idx += 1;
    }

    while left_idx < left.len() {
        values[out_idx] = left[left_idx].clone();
        left_idx += 1;
        out_idx += 1;
    }

    while right_idx < right.len() {
        values[out_idx] = right[right_idx].clone();
        right_idx += 1;
        out_idx += 1;
    }

    Ok(())
}

unsafe extern "C" fn lua_table_getn(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return -1;
    };
    l.push_value(Value::Number(table.length() as f64));
    1
}

unsafe extern "C" fn lua_table_foreach(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return table_error(l, "bad argument #1 to 'foreach' (table expected)");
    };
    let callback = match l.at(2).cloned().unwrap_or(Value::Nil) {
        value @ Value::Function(_) => value,
        _ => return table_error(l, "bad argument #2 to 'foreach' (function expected)"),
    };
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return table_error(l, "foreach unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let mut key = Value::Nil;
    loop {
        let next = {
            // SAFETY: table argument remains reachable on the active Lua stack.
            let Some(table) = (unsafe { table_ref.as_ref() }) else {
                return table_error(l, "bad argument #1 to 'foreach' (table expected)");
            };
            table.next(&key)
        };
        let Some((next_key, next_value)) = next else {
            l.push_nil();
            return 1;
        };
        let mut has_result = false;
        match call_value_with_results(
            l,
            gc,
            callback.clone(),
            &[next_key.clone(), next_value],
            None,
            |l, _, results| {
                let result = results.first().cloned().unwrap_or(Value::Nil);
                if !result.is_nil() {
                    l.push_value(result);
                    has_result = true;
                }
            },
        ) {
            Ok(()) => {}
            Err(err) => return push_sort_error(l, TableSortError::from_runtime(err)),
        }
        if has_result {
            return 1;
        }
        key = next_key;
    }
}

unsafe extern "C" fn lua_table_foreachi(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
    };
    let callback = match l.at(2).cloned().unwrap_or(Value::Nil) {
        value @ Value::Function(_) => value,
        _ => return table_error(l, "bad argument #2 to 'foreachi' (function expected)"),
    };
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return table_error(l, "foreachi unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let len = {
        // SAFETY: table argument remains reachable on the active Lua stack.
        let Some(table) = (unsafe { table_ref.as_ref() }) else {
            return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
        };
        table_foreachi_limit(l, table)
    };

    for idx in 1..=len {
        let value = {
            // SAFETY: table argument remains reachable on the active Lua stack.
            let Some(table) = (unsafe { table_ref.as_ref() }) else {
                return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
            };
            table.get(&Value::Number(idx as f64))
        };
        let mut has_result = false;
        match call_value_with_results(
            l,
            gc,
            callback.clone(),
            &[Value::Number(idx as f64), value],
            None,
            |l, _, results| {
                let result = results.first().cloned().unwrap_or(Value::Nil);
                if !result.is_nil() {
                    l.push_value(result);
                    has_result = true;
                }
            },
        ) {
            Ok(()) => {}
            Err(err) => return push_sort_error(l, TableSortError::from_runtime(err)),
        }
        if has_result {
            return 1;
        }
    }

    l.push_nil();
    1
}

unsafe extern "C" fn lua_table_maxn(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return -1;
    };

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
    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
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
    let end = if nargs >= 4 {
        number_arg(l, 4).unwrap_or(table.length() as f64) as i32
    } else {
        table.length() as i32
    };

    let mut result = Vec::new();
    for idx in start..=end {
        if idx > start && append_concat_bytes(&mut result, &sep).is_err() {
            return table_error(l, "table.concat: result length overflow");
        }
        let value = table.get(&Value::Number(idx as f64));
        let Some(piece) = value_to_concat_bytes(l, &value) else {
            return -1;
        };
        if append_concat_bytes(&mut result, &piece).is_err() {
            return table_error(l, "table.concat: result length overflow");
        }
    }
    if push_lua_bytes(l, &result) { 1 } else { -1 }
}

unsafe extern "C" fn lua_table_sort(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return table_error(l, "bad argument #1 to 'table.sort' (table expected)");
    };

    let comparator = if nargs >= 2 {
        match l.at(2).cloned().unwrap_or(Value::Nil) {
            Value::Nil => None,
            value @ Value::Function(_) => Some(value),
            _ => return table_error(l, "bad argument #2 to 'table.sort' (function expected)"),
        }
    } else {
        None
    };

    let Some(gc_ptr) = l.active_gc_ptr() else {
        return table_error(l, "table.sort unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let mut values = {
        // SAFETY: table argument is on the active Lua stack.
        let Some(table) = (unsafe { table_ref.as_ref() }) else {
            return table_error(l, "bad argument #1 to 'table.sort' (table expected)");
        };
        let len = table.length();
        let mut values = Vec::with_capacity(len);
        for idx in 1..=len {
            values.push(table.get(&Value::Number(idx as f64)));
        }
        values
    };

    if let Err(err) = merge_sort_values(l, gc, &mut values, &comparator) {
        return push_sort_error(l, err);
    }

    if gc
        .with_mut(table_ref, |table| {
            for (idx, value) in values.iter().enumerate() {
                table.set(&Value::Number((idx + 1) as f64), value);
            }
        })
        .is_err()
    {
        return table_error(l, "invalid table");
    }
    0
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

        lua_vm::with_vm_context_parts(&mut state, &mut gc, &mut string_pool, |state, gc, _| {
            assert!(matches!(default_less(state, gc, &left, &right), Ok(true)));
        });
    }
}

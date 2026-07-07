//! 表库 (Table Library)
#![allow(clippy::not_unsafe_ptr_arg_deref)]
//!

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::RuntimeError;
use lua_vm::execute::call_value;
use lua_vm::state::LuaState;

pub fn open_table(l: &mut LuaState, gc: &mut GarbageCollector) {
    let table_lib = find_lib_table(l, "table");
    if table_lib.is_null() {
        return;
    }

    let table_ptr = table_lib.as_ptr() as *mut Table;
    reg(gc, table_ptr, "concat", lua_table_concat);
    reg(gc, table_ptr, "foreach", lua_table_foreach);
    reg(gc, table_ptr, "foreachi", lua_table_foreachi);
    reg(gc, table_ptr, "getn", lua_table_getn);
    reg(gc, table_ptr, "insert", lua_table_insert);
    reg(gc, table_ptr, "maxn", lua_table_maxn);
    reg(gc, table_ptr, "remove", lua_table_remove);
    reg(gc, table_ptr, "sort", lua_table_sort);
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

unsafe fn to_lua(l_ptr: *mut std::ffi::c_void) -> &'static mut LuaState {
    // SAFETY: l_ptr is passed by the VM CALL handler and points to the active LuaState.
    unsafe { &mut *(l_ptr as *mut LuaState) }
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

fn string_arg(l: &LuaState, idx: i32) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: string argument is on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        Some(Value::Number(n)) => Some(number_to_lua_string(*n)),
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

fn value_to_concat_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            // SAFETY: table value is reachable while the table is on the Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        Value::Number(n) => Some(number_to_lua_string(*n)),
        _ => None,
    }
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    let Some(gc_ptr) = l.gc else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let string_ref = if let Some(pool_ptr) = l.string_pool {
        // SAFETY: string_pool is installed from a live StringPool owned by the host.
        let pool = unsafe { &mut *pool_ptr };
        pool.find(text).unwrap_or_else(|| pool.intern(gc, text))
    } else {
        gc.create(GcString::new(text))
    };
    l.push_value(Value::String(string_ref));
    true
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

fn table_foreachi_limit(table: &Table) -> usize {
    for (key, value) in table.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: key is held by the table being inspected.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
            && key_string.data() == "n"
            && let Value::Number(n) = value
            && *n >= 0.0
        {
            return *n as usize;
        }
    }
    table.length()
}

fn string_data(value: &Value) -> Option<&str> {
    let Value::String(string_ref) = value else {
        return None;
    };
    // SAFETY: the string value is reachable through the active Lua stack, a
    // table, or a temporary sort buffer whose source table remains reachable.
    unsafe { string_ref.as_ref() }.map(|string| string.data())
}

fn value_metatable(value: &Value) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table_ref) => {
            // SAFETY: table values being compared are reachable during sorting.
            unsafe { table_ref.as_ref() }.and_then(|table| table.metatable())
        }
        Value::Userdata(userdata_ref) => {
            // SAFETY: userdata values being compared are reachable during sorting.
            unsafe { userdata_ref.as_ref() }.and_then(|userdata| userdata.metatable())
        }
        _ => None,
    }
}

fn lookup_metamethod(metatable: GcRef<Table>, name: &str) -> Option<Value> {
    // SAFETY: metatable references are held by values being compared.
    let metatable = unsafe { metatable.as_ref() }?;
    for (key, value) in metatable.hash_entries() {
        if string_data(key).is_some_and(|key| key == name) && !value.is_nil() {
            return Some(value.clone());
        }
    }
    None
}

fn find_less_metamethod(lhs: &Value, rhs: &Value) -> Option<Value> {
    value_metatable(lhs)
        .and_then(|mt| lookup_metamethod(mt, "__lt"))
        .or_else(|| value_metatable(rhs).and_then(|mt| lookup_metamethod(mt, "__lt")))
}

fn compare_with_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    comparator: &Value,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, TableSortError> {
    let results = call_value(
        l,
        gc,
        comparator.clone(),
        &[lhs.clone(), rhs.clone()],
        Some(1),
    )
    .map_err(TableSortError::from_runtime)?;
    Ok(results.first().is_some_and(is_truthy))
}

fn default_less(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, TableSortError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a < b),
        (Value::String(_), Value::String(_)) => {
            let lhs = string_data(lhs).unwrap_or_default();
            let rhs = string_data(rhs).unwrap_or_default();
            Ok(lhs < rhs)
        }
        _ => {
            if let Some(metamethod) = find_less_metamethod(lhs, rhs) {
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
    let l = unsafe { to_lua(l_ptr) };
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
    let l = unsafe { to_lua(l_ptr) };
    let Some(table_ref) = table_arg(l, 1) else {
        return table_error(l, "bad argument #1 to 'foreach' (table expected)");
    };
    let callback = match l.at(2).cloned().unwrap_or(Value::Nil) {
        value @ Value::Function(_) => value,
        _ => return table_error(l, "bad argument #2 to 'foreach' (function expected)"),
    };
    let Some(gc_ptr) = l.gc else {
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
        let results = match call_value(
            l,
            gc,
            callback.clone(),
            &[next_key.clone(), next_value],
            None,
        ) {
            Ok(results) => results,
            Err(err) => return push_sort_error(l, TableSortError::from_runtime(err)),
        };
        let result = results.first().cloned().unwrap_or(Value::Nil);
        if !result.is_nil() {
            l.push_value(result);
            return 1;
        }
        key = next_key;
    }
}

unsafe extern "C" fn lua_table_foreachi(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(table_ref) = table_arg(l, 1) else {
        return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
    };
    let callback = match l.at(2).cloned().unwrap_or(Value::Nil) {
        value @ Value::Function(_) => value,
        _ => return table_error(l, "bad argument #2 to 'foreachi' (function expected)"),
    };
    let Some(gc_ptr) = l.gc else {
        return table_error(l, "foreachi unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let len = {
        // SAFETY: table argument remains reachable on the active Lua stack.
        let Some(table) = (unsafe { table_ref.as_ref() }) else {
            return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
        };
        table_foreachi_limit(table)
    };

    for idx in 1..=len {
        let value = {
            // SAFETY: table argument remains reachable on the active Lua stack.
            let Some(table) = (unsafe { table_ref.as_ref() }) else {
                return table_error(l, "bad argument #1 to 'foreachi' (table expected)");
            };
            table.get(&Value::Number(idx as f64))
        };
        let results = match call_value(
            l,
            gc,
            callback.clone(),
            &[Value::Number(idx as f64), value],
            None,
        ) {
            Ok(results) => results,
            Err(err) => return push_sort_error(l, TableSortError::from_runtime(err)),
        };
        let result = results.first().cloned().unwrap_or(Value::Nil);
        if !result.is_nil() {
            l.push_value(result);
            return 1;
        }
    }

    l.push_nil();
    1
}

unsafe extern "C" fn lua_table_maxn(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
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
    let l = unsafe { to_lua(l_ptr) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    if nargs < 2 {
        return -1;
    }

    let table_ptr = table_ref.as_ptr() as *mut Table;
    // SAFETY: table argument is on the active Lua stack and GC does not run here.
    let len = unsafe { (*table_ptr).length() as i32 };
    if nargs == 2 {
        let value = l.at(2).cloned().unwrap_or(Value::Nil);
        // SAFETY: table_ptr points to the active table argument.
        unsafe {
            (*table_ptr).set(&Value::Number((len + 1) as f64), &value);
        }
        return 0;
    }

    let Some(pos_num) = number_arg(l, 2) else {
        return -1;
    };
    let pos = pos_num as i32;
    let value = l.at(3).cloned().unwrap_or(Value::Nil);
    // SAFETY: table_ptr points to the active table argument.
    unsafe {
        for i in (pos..=len).rev() {
            let shifted = (*table_ptr).get(&Value::Number(i as f64));
            (*table_ptr).set(&Value::Number((i + 1) as f64), &shifted);
        }
        (*table_ptr).set(&Value::Number(pos as f64), &value);
    }
    0
}

unsafe extern "C" fn lua_table_remove(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    let table_ptr = table_ref.as_ptr() as *mut Table;
    // SAFETY: table argument is on the active Lua stack and GC does not run here.
    let len = unsafe { (*table_ptr).length() as i32 };
    let pos = if nargs >= 2 {
        number_arg(l, 2).unwrap_or(0.0) as i32
    } else {
        len
    };

    if pos < 1 || pos > len {
        l.push_value(Value::Nil);
        return 1;
    }

    // SAFETY: table_ptr points to the active table argument.
    let removed = unsafe { (*table_ptr).get(&Value::Number(pos as f64)) };
    // SAFETY: table_ptr points to the active table argument.
    unsafe {
        for i in pos..len {
            let shifted = (*table_ptr).get(&Value::Number((i + 1) as f64));
            (*table_ptr).set(&Value::Number(i as f64), &shifted);
        }
        (*table_ptr).remove(&Value::Number(len as f64));
    }
    l.push_value(removed);
    1
}

unsafe extern "C" fn lua_table_concat(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let nargs = l.get_top();
    let Some(table_ref) = table_arg(l, 1) else {
        return -1;
    };
    // SAFETY: table argument is on the active Lua stack.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return -1;
    };

    let sep = if nargs >= 2 {
        string_arg(l, 2).unwrap_or_default()
    } else {
        String::new()
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

    let mut pieces = Vec::new();
    for idx in start..=end {
        let value = table.get(&Value::Number(idx as f64));
        let Some(piece) = value_to_concat_string(&value) else {
            return -1;
        };
        pieces.push(piece);
    }
    let result = pieces.join(&sep);
    if push_lua_string(l, &result) { 1 } else { -1 }
}

unsafe extern "C" fn lua_table_sort(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
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

    let Some(gc_ptr) = l.gc else {
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

    let table_ptr = table_ref.as_ptr() as *mut Table;
    // SAFETY: table_ref points to the active table argument and VM execution is single-threaded.
    unsafe {
        for (idx, value) in values.iter().enumerate() {
            (*table_ptr).set(&Value::Number((idx + 1) as f64), value);
        }
    }
    0
}

//! 字符串库 (String Library)
#![allow(clippy::not_unsafe_ptr_arg_deref)]
//!

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::execute::call_value;
use lua_vm::state::LuaState;

pub fn open_string(l: &mut LuaState, gc: &mut GarbageCollector) {
    let string_lib = find_lib_table(l, "string");
    if string_lib.is_null() {
        return;
    }

    let table_ptr = string_lib.as_ptr() as *mut Table;
    reg(gc, table_ptr, "byte", lua_string_byte);
    reg(gc, table_ptr, "char", lua_string_char);
    reg(gc, table_ptr, "dump", lua_string_dump);
    reg(gc, table_ptr, "find", lua_string_find);
    reg(gc, table_ptr, "format", lua_string_format);
    reg_gmatch_aliases(gc, table_ptr);
    reg(gc, table_ptr, "gsub", lua_string_gsub);
    reg(gc, table_ptr, "len", lua_string_len);
    reg(gc, table_ptr, "lower", lua_string_lower);
    reg(gc, table_ptr, "match", lua_string_match);
    reg(gc, table_ptr, "rep", lua_string_rep);
    reg(gc, table_ptr, "reverse", lua_string_reverse);
    reg(gc, table_ptr, "sub", lua_string_sub);
    reg(gc, table_ptr, "upper", lua_string_upper);
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

fn reg_gmatch_aliases(gc: &mut GarbageCollector, table: *mut Table) {
    let func_obj = gc.create(Function::new_c(lua_string_gmatch));
    for name in ["gmatch", "gfind"] {
        let name_str = gc.create(GcString::new(name));
        // SAFETY: table points to the library table created and rooted by open_library.
        unsafe {
            (*table).set(&Value::String(name_str), &Value::Function(func_obj));
        }
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

fn string_arg(l: &LuaState, idx: i32) -> Option<Vec<u8>> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: string argument is on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| lua_bytes_from_str(s.data()))
        }
        Some(Value::Number(n)) => Some(number_to_lua_string(*n).into_bytes()),
        _ => None,
    }
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        Some(Value::String(s)) => {
            // SAFETY: string argument is on the active Lua stack.
            unsafe { s.as_ref() }.and_then(|s| s.data().parse::<f64>().ok())
        }
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

fn bytes_to_string(bytes: Vec<u8>) -> String {
    bytes.into_iter().map(char::from).collect()
}

fn lua_bytes_from_str(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if code <= 0xff {
            bytes.push(code as u8);
        } else {
            let mut buf = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }
    bytes
}

fn make_lua_string(l: &mut LuaState, text: &str) -> Option<GcRef<GcString>> {
    let gc_ptr = l.gc?;
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    Some(if let Some(pool_ptr) = l.string_pool {
        // SAFETY: string_pool is installed from a live StringPool owned by the host.
        let pool = unsafe { &mut *pool_ptr };
        pool.find(text).unwrap_or_else(|| pool.intern(gc, text))
    } else {
        gc.create(GcString::new(text))
    })
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    let Some(string_ref) = make_lua_string(l, text) else {
        return false;
    };
    l.push_value(Value::String(string_ref));
    true
}

fn lua_string_position(pos: i32, len: usize) -> i32 {
    if pos >= 0 {
        return pos;
    }

    let signed_len = len as i32;
    if pos < -signed_len {
        0
    } else {
        signed_len + pos + 1
    }
}

unsafe extern "C" fn lua_string_len(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(s) = string_arg(l, 1) else {
        return -1;
    };
    l.push_value(Value::Number(s.len() as f64));
    1
}

unsafe extern "C" fn lua_string_byte(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(s) = string_arg(l, 1) else {
        return -1;
    };

    let len = s.len();
    let start = number_arg(l, 2).map_or(1, |n| n as i32);
    let end = number_arg(l, 3).map_or(start, |n| n as i32);
    let start = lua_string_position(start, len).max(1);
    let end = lua_string_position(end, len).min(len as i32);

    if start > end || start > len as i32 || end < 1 {
        return 0;
    }

    let start_idx = (start - 1) as usize;
    let end_idx = end as usize;
    for byte in &s[start_idx..end_idx] {
        l.push_value(Value::Number(*byte as f64));
    }
    (end_idx - start_idx) as i32
}

unsafe extern "C" fn lua_string_char(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let nargs = l.get_top();
    let mut bytes = Vec::with_capacity(nargs.max(0) as usize);

    for idx in 1..=nargs {
        let Some(n) = number_arg(l, idx) else {
            return -1;
        };
        let byte = n as i32;
        if !(0..=255).contains(&byte) {
            return -1;
        }
        bytes.push(byte as u8);
    }

    if push_lua_string(l, &bytes_to_string(bytes)) {
        1
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_dump(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(Value::Function(func_ref)) = l.at(1).cloned() else {
        return string_format_error(l, "bad argument #1 to 'dump' (function expected)");
    };
    let Some(dumped) = crate::dump::dump_function(func_ref) else {
        return string_format_error(l, "unable to dump function");
    };

    if push_lua_string(l, &dumped) { 1 } else { -1 }
}

unsafe extern "C" fn lua_string_format(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(format_bytes) = string_arg(l, 1) else {
        return string_format_error(l, "bad argument #1 to 'format' (string expected)");
    };

    match format_lua_string(l, &format_bytes) {
        Ok(result) => {
            if push_lua_string(l, &result) {
                1
            } else {
                -1
            }
        }
        Err(message) => string_format_error(l, &message),
    }
}

unsafe extern "C" fn lua_string_find(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(source) = string_arg(l, 1) else {
        return string_format_error(l, "bad argument #1 to 'find' (string expected)");
    };
    let Some(pattern) = string_arg(l, 2) else {
        return string_format_error(l, "bad argument #2 to 'find' (string expected)");
    };
    if let Err(message) = validate_pattern(&pattern) {
        return string_format_error(l, &message);
    }

    let init = number_arg(l, 3).unwrap_or(1.0) as i32;
    let plain = l.at(4).is_some_and(|v| !v.is_false());
    let start_pos = lua_string_position(init, source.len()).max(1);
    let start_idx = (start_pos - 1).min(source.len() as i32) as usize;

    if pattern.is_empty() {
        if start_idx <= source.len() {
            l.push_value(Value::Number((start_idx + 1) as f64));
            l.push_value(Value::Number(start_idx as f64));
            return 2;
        }
        l.push_value(Value::Nil);
        return 1;
    }

    let found = if plain || pattern_has_no_magic(&pattern) {
        find_plain(&source, &pattern, start_idx).map(|(start, end)| PatternMatch {
            start,
            end,
            captures: Vec::new(),
        })
    } else {
        find_lua_pattern(&source, &pattern, start_idx)
    };

    if let Some(found) = found {
        l.push_value(Value::Number((found.start + 1) as f64));
        l.push_value(Value::Number(found.end as f64));
        let capture_count = found.captures.len();
        if !push_captures(l, &source, &found.captures) {
            return -1;
        }
        2 + capture_count as i32
    } else {
        l.push_value(Value::Nil);
        1
    }
}

unsafe extern "C" fn lua_string_match(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(source) = string_arg(l, 1) else {
        return string_format_error(l, "bad argument #1 to 'match' (string expected)");
    };
    let Some(pattern) = string_arg(l, 2) else {
        return string_format_error(l, "bad argument #2 to 'match' (string expected)");
    };
    if let Err(message) = validate_pattern(&pattern) {
        return string_format_error(l, &message);
    }

    let init = number_arg(l, 3).unwrap_or(1.0) as i32;
    let start_pos = lua_string_position(init, source.len()).max(1);
    let start_idx = (start_pos - 1).min(source.len() as i32) as usize;

    let found = if pattern.is_empty() {
        (start_idx <= source.len()).then_some(PatternMatch {
            start: start_idx,
            end: start_idx,
            captures: Vec::new(),
        })
    } else if pattern_has_no_magic(&pattern) {
        find_plain(&source, &pattern, start_idx).map(|(start, end)| PatternMatch {
            start,
            end,
            captures: Vec::new(),
        })
    } else {
        find_lua_pattern(&source, &pattern, start_idx)
    };

    let Some(found) = found else {
        l.push_value(Value::Nil);
        return 1;
    };

    if found.captures.is_empty() {
        let result = bytes_to_string(source[found.start..found.end].to_vec());
        if push_lua_string(l, &result) { 1 } else { -1 }
    } else if push_captures(l, &source, &found.captures) {
        found.captures.len() as i32
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_gsub(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(source) = string_arg(l, 1) else {
        return -1;
    };
    let Some(pattern) = string_arg(l, 2) else {
        return -1;
    };
    if let Err(message) = validate_pattern(&pattern) {
        return string_format_error(l, &message);
    }
    let replacement = l.at(3).cloned().unwrap_or(Value::Nil);
    if !matches!(
        replacement,
        Value::String(_) | Value::Number(_) | Value::Table(_) | Value::Function(_)
    ) {
        return string_format_error(
            l,
            "bad argument #3 to 'gsub' (string/function/table expected)",
        );
    }

    let max_replacements = number_arg(l, 4)
        .map(|n| if n <= 0.0 { 0 } else { n as usize })
        .unwrap_or(usize::MAX);

    let mut output = Vec::new();
    let mut count = 0usize;
    let mut cursor = 0usize;
    let mut search_start = 0usize;

    while count < max_replacements && search_start <= source.len() {
        let Some(found) = find_gsub_match(&source, &pattern, search_start) else {
            break;
        };

        if found.start < cursor {
            search_start = search_start.saturating_add(1);
            continue;
        }

        output.extend_from_slice(&source[cursor..found.start]);
        let Some(replacement_text) = gsub_replacement(l, &source, &found, &replacement) else {
            return -1;
        };
        output.extend_from_slice(&lua_bytes_from_str(&replacement_text));
        count += 1;

        if found.end == found.start {
            cursor = found.end;
            search_start = found.end + 1;
        } else {
            cursor = found.end;
            search_start = found.end;
        }
    }

    output.extend_from_slice(&source[cursor..]);
    let result = bytes_to_string(output);
    if !push_lua_string(l, &result) {
        return -1;
    }
    l.push_value(Value::Number(count as f64));
    2
}

unsafe extern "C" fn lua_string_gmatch(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(source) = string_arg(l, 1) else {
        return string_format_error(l, "bad argument #1 to 'gmatch' (string expected)");
    };
    let Some(pattern) = string_arg(l, 2) else {
        return string_format_error(l, "bad argument #2 to 'gmatch' (string expected)");
    };
    if let Err(message) = validate_pattern(&pattern) {
        return string_format_error(l, &message);
    }
    let Some(gc_ptr) = l.gc else {
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };

    let state_ref = gc.create(Table::new());
    let iter_ref = gc.create(Function::new_c(lua_string_gmatch_iter));
    let source_text = bytes_to_string(source);
    let pattern_text = bytes_to_string(pattern);
    let Some(source_ref) = make_lua_string(l, &source_text) else {
        return -1;
    };
    let Some(pattern_ref) = make_lua_string(l, &pattern_text) else {
        return -1;
    };

    // SAFETY: state_ref was just allocated and is still owned by the active GC.
    let state = unsafe { &mut *(state_ref.as_ptr() as *mut Table) };
    if set_state_value(l, state, "source", Value::String(source_ref)).is_none()
        || set_state_value(l, state, "pattern", Value::String(pattern_ref)).is_none()
        || set_state_value(l, state, "next", Value::Number(0.0)).is_none()
    {
        return -1;
    }

    l.push_value(Value::Function(iter_ref));
    l.push_value(Value::Table(state_ref));
    l.push_value(Value::Nil);
    3
}

unsafe extern "C" fn lua_string_gmatch_iter(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(Value::Table(state_ref)) = l.at(1).cloned() else {
        return 0;
    };
    // SAFETY: state table is held by the generic-for state register.
    let Some(state) = (unsafe { state_ref.as_ref() }) else {
        return 0;
    };

    let Some(source) = state_string_field(l, state, "source") else {
        return 0;
    };
    let Some(pattern) = state_string_field(l, state, "pattern") else {
        return 0;
    };
    let next = state_number_field(l, state, "next").unwrap_or(0.0).max(0.0) as usize;
    let Some(found) = find_gsub_match(&source, &pattern, next) else {
        return 0;
    };

    let new_next = if found.end == found.start {
        found.end.saturating_add(1)
    } else {
        found.end
    };
    // SAFETY: state table is held by the generic-for state register and VM is single-threaded.
    let state_mut = unsafe { &mut *(state_ref.as_ptr() as *mut Table) };
    if set_state_value(l, state_mut, "next", Value::Number(new_next as f64)).is_none() {
        return -1;
    }

    if found.captures.is_empty() {
        let result = bytes_to_string(source[found.start..found.end].to_vec());
        if push_lua_string(l, &result) { 1 } else { -1 }
    } else if push_captures(l, &source, &found.captures) {
        found.captures.len() as i32
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_sub(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(s) = string_arg(l, 1) else {
        return -1;
    };
    let Some(start_num) = number_arg(l, 2) else {
        return -1;
    };

    let len = s.len();
    let end_num = number_arg(l, 3).unwrap_or(len as f64);
    let mut start = lua_string_position(start_num as i32, len);
    let mut end = lua_string_position(end_num as i32, len);

    if start < 1 {
        start = 1;
    }
    if end > len as i32 {
        end = len as i32;
    }

    let result = if start > end {
        String::new()
    } else {
        let start_idx = (start - 1) as usize;
        let end_idx = end as usize;
        bytes_to_string(s[start_idx..end_idx].to_vec())
    };
    if push_lua_string(l, &result) { 1 } else { -1 }
}

fn find_plain(source: &[u8], needle: &[u8], start_idx: usize) -> Option<(usize, usize)> {
    if start_idx > source.len() || needle.len() > source.len().saturating_sub(start_idx) {
        return None;
    }

    source[start_idx..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| {
            let start = start_idx + offset;
            (start, start + needle.len())
        })
}

fn pattern_has_no_magic(pattern: &[u8]) -> bool {
    !pattern.iter().any(|b| {
        matches!(
            *b,
            b'.' | b'^' | b'$' | b'(' | b')' | b'%' | b'[' | b'*' | b'+' | b'-' | b'?'
        )
    })
}

#[derive(Clone, Debug, Default)]
struct FormatSpec {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
    width: Option<usize>,
    precision: Option<usize>,
    conv: u8,
}

fn format_lua_string(l: &LuaState, format: &[u8]) -> Result<String, String> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    let mut arg_idx = 2i32;

    while idx < format.len() {
        if format[idx] != b'%' {
            out.push(format[idx]);
            idx += 1;
            continue;
        }

        if format.get(idx + 1) == Some(&b'%') {
            out.push(b'%');
            idx += 2;
            continue;
        }

        let (spec, next_idx) = parse_format_spec(format, idx + 1)?;
        let arg = l
            .at(arg_idx)
            .cloned()
            .ok_or_else(|| "string.format: not enough arguments".to_string())?;
        arg_idx += 1;

        let piece = format_value_with_spec(&arg, &spec)?;
        out.extend_from_slice(&lua_bytes_from_str(&piece));
        idx = next_idx;
    }

    Ok(bytes_to_string(out))
}

fn parse_format_spec(format: &[u8], mut idx: usize) -> Result<(FormatSpec, usize), String> {
    let mut spec = FormatSpec::default();

    while let Some(flag) = format.get(idx).copied() {
        match flag {
            b'-' => spec.left = true,
            b'+' => spec.plus = true,
            b' ' => spec.space = true,
            b'#' => spec.alternate = true,
            b'0' => spec.zero = true,
            _ => break,
        }
        idx += 1;
    }

    let width_start = idx;
    while format.get(idx).is_some_and(|b| b.is_ascii_digit()) {
        idx += 1;
    }
    if idx > width_start {
        spec.width = std::str::from_utf8(&format[width_start..idx])
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
    }

    if format.get(idx) == Some(&b'.') {
        idx += 1;
        let precision_start = idx;
        while format.get(idx).is_some_and(|b| b.is_ascii_digit()) {
            idx += 1;
        }
        spec.precision = if idx > precision_start {
            std::str::from_utf8(&format[precision_start..idx])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
        } else {
            Some(0)
        };
    }

    while matches!(format.get(idx), Some(b'h' | b'l' | b'L')) {
        idx += 1;
    }

    let Some(conv) = format.get(idx).copied() else {
        return Err("string.format: incomplete format option".to_string());
    };
    spec.conv = conv;
    if !matches!(
        conv,
        b'c' | b'd'
            | b'i'
            | b'o'
            | b'u'
            | b'x'
            | b'X'
            | b'e'
            | b'E'
            | b'f'
            | b'g'
            | b'G'
            | b'q'
            | b's'
    ) {
        return Err(format!(
            "invalid option '%{}' to 'format'",
            char::from(conv)
        ));
    }

    Ok((spec, idx + 1))
}

fn format_value_with_spec(value: &Value, spec: &FormatSpec) -> Result<String, String> {
    match spec.conv {
        b's' => Ok(apply_string_precision_and_width(
            value_to_format_string(value),
            spec,
        )),
        b'q' => Ok(lua_quote_string(&value_to_format_bytes(value))),
        b'c' => {
            let n = numeric_value(value)? as u32;
            let ch = char::from_u32(n).unwrap_or('\0').to_string();
            Ok(apply_width(ch, spec, false))
        }
        b'd' | b'i' => Ok(format_integer(
            numeric_value(value)? as i64,
            10,
            false,
            spec,
        )),
        b'u' => Ok(format_unsigned_integer(
            numeric_value(value)? as u64,
            10,
            false,
            spec,
        )),
        b'o' => Ok(format_unsigned_integer(
            numeric_value(value)? as u64,
            8,
            false,
            spec,
        )),
        b'x' => Ok(format_unsigned_integer(
            numeric_value(value)? as u64,
            16,
            false,
            spec,
        )),
        b'X' => Ok(format_unsigned_integer(
            numeric_value(value)? as u64,
            16,
            true,
            spec,
        )),
        b'f' => Ok(format_float_fixed(numeric_value(value)?, spec)),
        b'e' | b'E' => Ok(format_float_exp(numeric_value(value)?, spec)),
        b'g' | b'G' => Ok(format_float_general(numeric_value(value)?, spec)),
        _ => Err(format!(
            "invalid option '%{}' to 'format'",
            char::from(spec.conv)
        )),
    }
}

fn string_format_error(l: &mut LuaState, message: &str) -> i32 {
    if !push_lua_string(l, message) {
        return -1;
    }
    -1
}

fn value_to_format_string(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => number_to_lua_string(*n),
        Value::String(s) => {
            // SAFETY: value is an argument on the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| s.data().to_string())
                .unwrap_or_default()
        }
        Value::Table(t) => format!("table: {:p}", t.as_ptr()),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => format!("lightuserdata: {:p}", p.as_ptr()),
    }
}

fn value_to_format_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::String(s) => {
            // SAFETY: value is an argument on the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| lua_bytes_from_str(s.data()))
                .unwrap_or_default()
        }
        _ => value_to_format_string(value).into_bytes(),
    }
}

fn numeric_value(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::String(s) => {
            // SAFETY: value is an argument on the active Lua stack.
            unsafe { s.as_ref() }
                .and_then(|s| s.data().trim().parse::<f64>().ok())
                .ok_or_else(|| "string.format: number expected".to_string())
        }
        _ => Err("string.format: number expected".to_string()),
    }
}

fn apply_string_precision_and_width(mut value: String, spec: &FormatSpec) -> String {
    if let Some(precision) = spec.precision {
        value = bytes_to_string(value.into_bytes().into_iter().take(precision).collect());
    }
    apply_width(value, spec, false)
}

fn apply_width(value: String, spec: &FormatSpec, zero_after_sign: bool) -> String {
    let Some(width) = spec.width else {
        return value;
    };
    let len = value.len();
    if len >= width {
        return value;
    }

    let pad_len = width - len;
    if spec.left {
        return format!("{}{}", value, " ".repeat(pad_len));
    }

    if spec.zero && zero_after_sign {
        let split = if matches!(value.as_bytes().first(), Some(b'-' | b'+' | b' ')) {
            1
        } else if value.starts_with("0x") || value.starts_with("0X") {
            2
        } else {
            0
        };
        let (prefix, rest) = value.split_at(split);
        format!("{}{}{}", prefix, "0".repeat(pad_len), rest)
    } else {
        format!("{}{}", " ".repeat(pad_len), value)
    }
}

fn format_integer(value: i64, radix: u32, uppercase: bool, spec: &FormatSpec) -> String {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let mut digits = integer_digits(magnitude, radix, uppercase);
    if let Some(precision) = spec.precision {
        if digits == "0" && precision == 0 {
            digits.clear();
        }
        if digits.len() < precision {
            digits = format!("{}{}", "0".repeat(precision - digits.len()), digits);
        }
    }

    let mut prefix = String::new();
    if negative {
        prefix.push('-');
    } else if spec.plus {
        prefix.push('+');
    } else if spec.space {
        prefix.push(' ');
    }
    prefix.push_str(&alternate_prefix(
        radix,
        uppercase,
        spec.alternate,
        magnitude,
    ));

    let value = format!("{prefix}{digits}");
    apply_width(
        value,
        spec,
        spec.zero && spec.precision.is_none() && !spec.left,
    )
}

fn format_unsigned_integer(value: u64, radix: u32, uppercase: bool, spec: &FormatSpec) -> String {
    let mut digits = integer_digits(value, radix, uppercase);
    if let Some(precision) = spec.precision {
        if digits == "0" && precision == 0 {
            digits.clear();
        }
        if digits.len() < precision {
            digits = format!("{}{}", "0".repeat(precision - digits.len()), digits);
        }
    }
    let prefix = alternate_prefix(radix, uppercase, spec.alternate, value);
    apply_width(
        format!("{prefix}{digits}"),
        spec,
        spec.zero && spec.precision.is_none() && !spec.left,
    )
}

fn integer_digits(value: u64, radix: u32, uppercase: bool) -> String {
    match (radix, uppercase) {
        (8, _) => format!("{value:o}"),
        (16, true) => format!("{value:X}"),
        (16, false) => format!("{value:x}"),
        _ => value.to_string(),
    }
}

fn alternate_prefix(radix: u32, uppercase: bool, alternate: bool, value: u64) -> String {
    if !alternate || value == 0 {
        return String::new();
    }
    match (radix, uppercase) {
        (8, _) => "0".to_string(),
        (16, true) => "0X".to_string(),
        (16, false) => "0x".to_string(),
        _ => String::new(),
    }
}

fn format_float_fixed(value: f64, spec: &FormatSpec) -> String {
    let precision = spec.precision.unwrap_or(6);
    let mut result = format!("{:.*}", precision, value.abs());
    if spec.alternate && !result.contains('.') {
        result.push('.');
    }
    result = add_float_sign(value, result, spec);
    apply_width(result, spec, spec.zero && !spec.left)
}

fn format_float_exp(value: f64, spec: &FormatSpec) -> String {
    let precision = spec.precision.unwrap_or(6);
    let mut result = normalize_exponent(format!("{:.*e}", precision, value.abs()), spec.conv);
    result = add_float_sign(value, result, spec);
    apply_width(result, spec, spec.zero && !spec.left)
}

fn format_float_general(value: f64, spec: &FormatSpec) -> String {
    let precision = spec.precision.unwrap_or(6).max(1);
    let abs = value.abs();
    let exponent = if abs == 0.0 {
        0
    } else {
        abs.log10().floor() as i32
    };

    let mut result = if exponent < -4 || exponent >= precision as i32 {
        normalize_exponent(format!("{:.*e}", precision - 1, abs), spec.conv)
    } else {
        let decimals = (precision as i32 - exponent - 1).max(0) as usize;
        format!("{:.*}", decimals, abs)
    };

    if !spec.alternate {
        trim_float_trailing_zeroes(&mut result);
    }
    if matches!(spec.conv, b'G') {
        result = result.to_ascii_uppercase();
    }
    result = add_float_sign(value, result, spec);
    apply_width(result, spec, spec.zero && !spec.left)
}

fn add_float_sign(value: f64, mut result: String, spec: &FormatSpec) -> String {
    if value.is_sign_negative() {
        result.insert(0, '-');
    } else if spec.plus {
        result.insert(0, '+');
    } else if spec.space {
        result.insert(0, ' ');
    }
    result
}

fn normalize_exponent(mut value: String, conv: u8) -> String {
    let Some(pos) = value.find('e') else {
        return value;
    };
    let mantissa = value[..pos].to_string();
    let exp: i32 = value[pos + 1..].parse().unwrap_or(0);
    let marker = if conv == b'E' || conv == b'G' {
        'E'
    } else {
        'e'
    };
    value = format!("{mantissa}{marker}{exp:+03}");
    value
}

fn trim_float_trailing_zeroes(value: &mut String) {
    let exponent_pos = value.find(['e', 'E']);
    let exponent = exponent_pos.map(|pos| value.split_off(pos));
    if value.contains('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
    }
    if let Some(exponent) = exponent {
        value.push_str(&exponent);
    }
}

fn lua_quote_string(bytes: &[u8]) -> String {
    let mut out = Vec::new();
    out.push(b'"');
    for &byte in bytes {
        match byte {
            b'"' | b'\\' => {
                out.push(b'\\');
                out.push(byte);
            }
            b'\n' => {
                out.push(b'\\');
                out.push(b'\n');
            }
            0..=31 | 127 => out.extend_from_slice(format!("\\{byte:03}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
    bytes_to_string(out)
}

fn find_gsub_match(source: &[u8], pattern: &[u8], start_idx: usize) -> Option<PatternMatch> {
    if pattern.is_empty() {
        return (start_idx <= source.len()).then_some(PatternMatch {
            start: start_idx,
            end: start_idx,
            captures: Vec::new(),
        });
    }

    if pattern_has_no_magic(pattern) {
        find_plain(source, pattern, start_idx).map(|(start, end)| PatternMatch {
            start,
            end,
            captures: Vec::new(),
        })
    } else {
        find_lua_pattern(source, pattern, start_idx)
    }
}

fn set_state_value(l: &mut LuaState, state: &mut Table, key: &str, value: Value) -> Option<()> {
    let key_ref = make_lua_string(l, key)?;
    state.set(&Value::String(key_ref), &value);
    Some(())
}

fn state_value(l: &mut LuaState, state: &Table, key: &str) -> Option<Value> {
    let key_ref = make_lua_string(l, key)?;
    Some(state.get(&Value::String(key_ref)))
}

fn state_string_field(l: &mut LuaState, state: &Table, key: &str) -> Option<Vec<u8>> {
    match state_value(l, state, key)? {
        Value::String(s) => {
            // SAFETY: gmatch state table keeps the string reachable while iterating.
            unsafe { s.as_ref() }.map(|s| lua_bytes_from_str(s.data()))
        }
        _ => None,
    }
}

fn state_number_field(l: &mut LuaState, state: &Table, key: &str) -> Option<f64> {
    match state_value(l, state, key)? {
        Value::Number(n) => Some(n),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct PatternMatch {
    start: usize,
    end: usize,
    captures: Vec<Capture>,
}

#[derive(Clone, Debug)]
enum Capture {
    Open(usize),
    Range(usize, usize),
    Position(usize),
}

#[derive(Clone, Copy, Debug)]
enum PatternAtom {
    Any,
    Literal(u8),
    Class(u8),
    Set {
        start: usize,
        end: usize,
        negated: bool,
    },
    Balanced {
        open: u8,
        close: u8,
    },
    Frontier {
        start: usize,
        end: usize,
        negated: bool,
    },
    BackRef(usize),
}

fn find_lua_pattern(source: &[u8], pattern: &[u8], start_idx: usize) -> Option<PatternMatch> {
    if start_idx > source.len() {
        return None;
    }

    let anchored = pattern.first() == Some(&b'^');
    let pat_start = usize::from(anchored);
    if anchored {
        if start_idx > 0 {
            return None;
        }
        return match_lua_pattern_at(source, pattern, 0, pat_start).map(|(end, captures)| {
            PatternMatch {
                start: 0,
                end,
                captures,
            }
        });
    }

    (start_idx..=source.len()).find_map(|start| {
        match_lua_pattern_at(source, pattern, start, pat_start).map(|(end, captures)| {
            PatternMatch {
                start,
                end,
                captures,
            }
        })
    })
}

fn match_lua_pattern_at(
    source: &[u8],
    pattern: &[u8],
    source_idx: usize,
    pattern_idx: usize,
) -> Option<(usize, Vec<Capture>)> {
    match_pattern_from(source, pattern, source_idx, pattern_idx, Vec::new()).and_then(
        |(end, captures)| {
            if captures.iter().any(|cap| matches!(cap, Capture::Open(_))) {
                None
            } else {
                Some((end, captures))
            }
        },
    )
}

fn match_pattern_from(
    source: &[u8],
    pattern: &[u8],
    source_idx: usize,
    pattern_idx: usize,
    captures: Vec<Capture>,
) -> Option<(usize, Vec<Capture>)> {
    if pattern_idx >= pattern.len() {
        return Some((source_idx, captures));
    }

    match pattern[pattern_idx] {
        b'$' if pattern_idx + 1 == pattern.len() => {
            if source_idx == source.len() {
                Some((source_idx, captures))
            } else {
                None
            }
        }
        b'(' => {
            if pattern.get(pattern_idx + 1) == Some(&b')') {
                let mut next_captures = captures;
                next_captures.push(Capture::Position(source_idx));
                match_pattern_from(source, pattern, source_idx, pattern_idx + 2, next_captures)
            } else {
                let mut next_captures = captures;
                next_captures.push(Capture::Open(source_idx));
                match_pattern_from(source, pattern, source_idx, pattern_idx + 1, next_captures)
            }
        }
        b')' => {
            let mut next_captures = captures;
            let open_idx = next_captures
                .iter()
                .rposition(|cap| matches!(cap, Capture::Open(_)))?;
            let Capture::Open(start) = next_captures[open_idx] else {
                return None;
            };
            next_captures[open_idx] = Capture::Range(start, source_idx);
            match_pattern_from(source, pattern, source_idx, pattern_idx + 1, next_captures)
        }
        _ => {
            let (atom, next_pattern_idx) = parse_pattern_atom(pattern, pattern_idx)?;
            let suffix = pattern
                .get(next_pattern_idx)
                .copied()
                .filter(|b| matches!(*b, b'*' | b'+' | b'?' | b'-'));
            if let Some(suffix) = suffix {
                match_repeated_atom(
                    source,
                    pattern,
                    source_idx,
                    next_pattern_idx + 1,
                    captures,
                    atom,
                    suffix,
                )
            } else {
                let next_source_idx =
                    match_atom_once(source, source_idx, pattern, atom, &captures)?;
                match_pattern_from(source, pattern, next_source_idx, next_pattern_idx, captures)
            }
        }
    }
}

fn parse_pattern_atom(pattern: &[u8], pattern_idx: usize) -> Option<(PatternAtom, usize)> {
    let byte = *pattern.get(pattern_idx)?;
    match byte {
        b'.' => Some((PatternAtom::Any, pattern_idx + 1)),
        b'%' => {
            let escaped = *pattern.get(pattern_idx + 1)?;
            if escaped == b'b' {
                Some((
                    PatternAtom::Balanced {
                        open: *pattern.get(pattern_idx + 2)?,
                        close: *pattern.get(pattern_idx + 3)?,
                    },
                    pattern_idx + 4,
                ))
            } else if escaped == b'f' && pattern.get(pattern_idx + 2) == Some(&b'[') {
                match parse_pattern_set(pattern, pattern_idx + 2)? {
                    (
                        PatternAtom::Set {
                            start,
                            end,
                            negated,
                        },
                        next_idx,
                    ) => Some((
                        PatternAtom::Frontier {
                            start,
                            end,
                            negated,
                        },
                        next_idx,
                    )),
                    _ => None,
                }
            } else if escaped.is_ascii_digit() && escaped != b'0' {
                Some((
                    PatternAtom::BackRef((escaped - b'0') as usize),
                    pattern_idx + 2,
                ))
            } else if is_lua_class(escaped) {
                Some((PatternAtom::Class(escaped), pattern_idx + 2))
            } else {
                Some((PatternAtom::Literal(escaped), pattern_idx + 2))
            }
        }
        b'[' => parse_pattern_set(pattern, pattern_idx),
        _ => Some((PatternAtom::Literal(byte), pattern_idx + 1)),
    }
}

fn validate_pattern(pattern: &[u8]) -> Result<usize, String> {
    let mut idx = 0usize;
    let mut open_captures = 0usize;
    let mut closed_captures = 0usize;

    while idx < pattern.len() {
        match pattern[idx] {
            b'(' => {
                if pattern.get(idx + 1) == Some(&b')') {
                    closed_captures += 1;
                    idx += 2;
                } else {
                    open_captures += 1;
                    idx += 1;
                }
            }
            b')' => {
                if open_captures == 0 {
                    return Err("invalid pattern capture".to_string());
                }
                open_captures -= 1;
                closed_captures += 1;
                idx += 1;
            }
            _ => {
                idx = validate_pattern_atom(pattern, idx, closed_captures)?;
                if pattern
                    .get(idx)
                    .is_some_and(|suffix| matches!(*suffix, b'*' | b'+' | b'?' | b'-'))
                {
                    idx += 1;
                }
            }
        }
    }

    if open_captures != 0 {
        return Err("unfinished capture".to_string());
    }

    Ok(closed_captures)
}

fn validate_pattern_atom(
    pattern: &[u8],
    pattern_idx: usize,
    closed_captures: usize,
) -> Result<usize, String> {
    match pattern.get(pattern_idx).copied() {
        Some(b'%') => {
            let escaped = pattern
                .get(pattern_idx + 1)
                .copied()
                .ok_or_else(|| "malformed pattern (ends with '%')".to_string())?;
            match escaped {
                b'0' => Err("invalid capture index".to_string()),
                b'1'..=b'9' => {
                    let capture_idx = (escaped - b'0') as usize;
                    if capture_idx > closed_captures {
                        Err("invalid capture index".to_string())
                    } else {
                        Ok(pattern_idx + 2)
                    }
                }
                b'b' => {
                    if pattern_idx + 3 < pattern.len() {
                        Ok(pattern_idx + 4)
                    } else {
                        Err("malformed pattern (missing arguments to '%b')".to_string())
                    }
                }
                b'f' => {
                    if pattern.get(pattern_idx + 2) != Some(&b'[') {
                        return Err("missing '[' after '%f' in pattern".to_string());
                    }
                    parse_pattern_set(pattern, pattern_idx + 2)
                        .map(|(_, next_idx)| next_idx)
                        .ok_or_else(|| "malformed pattern set".to_string())
                }
                _ => Ok(pattern_idx + 2),
            }
        }
        Some(b'[') => parse_pattern_set(pattern, pattern_idx)
            .map(|(_, next_idx)| next_idx)
            .ok_or_else(|| "malformed pattern set".to_string()),
        Some(_) => Ok(pattern_idx + 1),
        None => Err("malformed pattern".to_string()),
    }
}

fn parse_pattern_set(pattern: &[u8], pattern_idx: usize) -> Option<(PatternAtom, usize)> {
    let mut idx = pattern_idx + 1;
    let negated = pattern.get(idx) == Some(&b'^');
    if negated {
        idx += 1;
    }
    let set_start = idx;
    while idx < pattern.len() {
        if pattern[idx] == b']' && idx > set_start {
            return Some((
                PatternAtom::Set {
                    start: set_start,
                    end: idx,
                    negated,
                },
                idx + 1,
            ));
        }
        if pattern[idx] == b'%' && idx + 1 < pattern.len() {
            idx += 2;
        } else {
            idx += 1;
        }
    }
    None
}

fn match_repeated_atom(
    source: &[u8],
    pattern: &[u8],
    source_idx: usize,
    next_pattern_idx: usize,
    captures: Vec<Capture>,
    atom: PatternAtom,
    suffix: u8,
) -> Option<(usize, Vec<Capture>)> {
    if suffix == b'?' {
        if let Some(next_idx) = match_atom_once(source, source_idx, pattern, atom, &captures)
            && next_idx != source_idx
            && let Some(result) = match_pattern_from(
                source,
                pattern,
                next_idx,
                next_pattern_idx,
                captures.clone(),
            )
        {
            return Some(result);
        }
        return match_pattern_from(source, pattern, source_idx, next_pattern_idx, captures);
    }

    let mut positions = vec![source_idx];
    let mut current_idx = source_idx;
    while let Some(next_idx) = match_atom_once(source, current_idx, pattern, atom, &captures) {
        if next_idx == current_idx {
            break;
        }
        positions.push(next_idx);
        current_idx = next_idx;
    }

    match suffix {
        b'+' => {
            if positions.len() <= 1 {
                return None;
            }
            positions[1..].iter().rev().find_map(|next_idx| {
                match_pattern_from(
                    source,
                    pattern,
                    *next_idx,
                    next_pattern_idx,
                    captures.clone(),
                )
            })
        }
        b'*' => positions.iter().rev().find_map(|next_idx| {
            match_pattern_from(
                source,
                pattern,
                *next_idx,
                next_pattern_idx,
                captures.clone(),
            )
        }),
        b'-' => positions.iter().find_map(|next_idx| {
            match_pattern_from(
                source,
                pattern,
                *next_idx,
                next_pattern_idx,
                captures.clone(),
            )
        }),
        _ => None,
    }
}

fn match_atom_once(
    source: &[u8],
    source_idx: usize,
    pattern: &[u8],
    atom: PatternAtom,
    captures: &[Capture],
) -> Option<usize> {
    match atom {
        PatternAtom::Balanced { open, close } => {
            if source.get(source_idx).copied() != Some(open) {
                return None;
            }
            let mut depth = 1usize;
            let mut idx = source_idx + 1;
            while idx < source.len() {
                let byte = source[idx];
                if byte == close {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(idx + 1);
                    }
                } else if byte == open {
                    depth += 1;
                }
                idx += 1;
            }
            None
        }
        PatternAtom::Frontier {
            start,
            end,
            negated,
        } => {
            let previous = if source_idx == 0 {
                0
            } else {
                *source.get(source_idx - 1)?
            };
            let current = source.get(source_idx).copied().unwrap_or(0);
            if !byte_matches_pattern_set(previous, pattern, start, end, negated)
                && byte_matches_pattern_set(current, pattern, start, end, negated)
            {
                Some(source_idx)
            } else {
                None
            }
        }
        PatternAtom::BackRef(capture_idx) => {
            let Capture::Range(start, end) = captures.get(capture_idx.checked_sub(1)?)? else {
                return None;
            };
            let capture = source.get(*start..*end)?;
            if source.get(source_idx..source_idx + capture.len()) == Some(capture) {
                Some(source_idx + capture.len())
            } else {
                None
            }
        }
        _ => {
            let byte = *source.get(source_idx)?;
            if pattern_atom_matches(byte, pattern, atom) {
                Some(source_idx + 1)
            } else {
                None
            }
        }
    }
}

fn pattern_atom_matches(byte: u8, pattern: &[u8], atom: PatternAtom) -> bool {
    match atom {
        PatternAtom::Any => true,
        PatternAtom::Literal(expected) => byte == expected,
        PatternAtom::Class(class) => byte_matches_class(byte, class),
        PatternAtom::Set {
            start,
            end,
            negated,
        } => byte_matches_pattern_set(byte, pattern, start, end, negated),
        PatternAtom::Balanced { .. } | PatternAtom::Frontier { .. } => false,
        PatternAtom::BackRef(_) => false,
    }
}

fn byte_matches_pattern_set(
    byte: u8,
    pattern: &[u8],
    start: usize,
    end: usize,
    negated: bool,
) -> bool {
    let matched = byte_matches_set(byte, pattern, start, end);
    if negated { !matched } else { matched }
}

fn byte_matches_set(byte: u8, pattern: &[u8], start: usize, end: usize) -> bool {
    let mut idx = start;
    while idx < end {
        if pattern[idx] == b'%' && idx + 1 < end {
            let class = pattern[idx + 1];
            if is_lua_class(class) {
                if byte_matches_class(byte, class) {
                    return true;
                }
            } else if byte == class {
                return true;
            }
            idx += 2;
            continue;
        }

        if idx + 2 < end && pattern[idx + 1] == b'-' && pattern[idx + 2] != b']' {
            if byte >= pattern[idx] && byte <= pattern[idx + 2] {
                return true;
            }
            idx += 3;
        } else {
            if byte == pattern[idx] {
                return true;
            }
            idx += 1;
        }
    }
    false
}

fn is_lua_class(class: u8) -> bool {
    matches!(
        class.to_ascii_lowercase(),
        b'a' | b'c' | b'd' | b'l' | b'p' | b's' | b'u' | b'w' | b'x' | b'z'
    )
}

fn byte_matches_class(byte: u8, class: u8) -> bool {
    let matched = match class.to_ascii_lowercase() {
        b'a' => byte.is_ascii_alphabetic(),
        b'c' => byte.is_ascii_control(),
        b'd' => byte.is_ascii_digit(),
        b'l' => byte.is_ascii_lowercase(),
        b'p' => byte.is_ascii_punctuation(),
        b's' => byte.is_ascii_whitespace(),
        b'u' => byte.is_ascii_uppercase(),
        b'w' => byte.is_ascii_alphanumeric(),
        b'x' => byte.is_ascii_hexdigit(),
        b'z' => byte == 0,
        _ => false,
    };

    if class.is_ascii_uppercase() {
        !matched
    } else {
        matched
    }
}

fn gsub_replacement(
    l: &mut LuaState,
    source: &[u8],
    found: &PatternMatch,
    replacement: &Value,
) -> Option<String> {
    match replacement {
        Value::String(template_ref) => {
            // SAFETY: replacement argument is on the active Lua stack.
            let template = lua_bytes_from_str(unsafe { template_ref.as_ref() }?.data());
            match expand_replacement_template(&template, source, found) {
                Ok(text) => Some(text),
                Err(message) => {
                    push_lua_string(l, &message);
                    None
                }
            }
        }
        Value::Number(n) => Some(number_to_lua_string(*n)),
        Value::Table(table_ref) => {
            let key = gsub_table_key(l, source, found)?;
            let value = gsub_table_replacement_value(l, *table_ref, &key)?;
            replacement_value_to_string(source, found, &value)
        }
        Value::Function(_) => gsub_function_replacement(l, source, found, replacement),
        _ => None,
    }
}

fn gsub_function_replacement(
    l: &mut LuaState,
    source: &[u8],
    found: &PatternMatch,
    replacement: &Value,
) -> Option<String> {
    let Some(gc_ptr) = l.gc else {
        push_lua_string(l, "gsub unavailable without an active GC");
        return None;
    };
    let args = gsub_function_args(l, source, found)?;
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    let results = match call_value(l, gc, replacement.clone(), &args, Some(1)) {
        Ok(results) => results,
        Err(err) => {
            if let Some(error_value) = err.error_value() {
                l.push_value(error_value);
            } else {
                push_lua_string(l, &err.message);
            }
            return None;
        }
    };
    let value = results.first().cloned().unwrap_or(Value::Nil);
    if let Some(text) = replacement_value_to_string(source, found, &value) {
        return Some(text);
    }

    push_lua_string(l, "invalid replacement value (a string expected)");
    None
}

fn gsub_function_args(l: &mut LuaState, source: &[u8], found: &PatternMatch) -> Option<Vec<Value>> {
    if found.captures.is_empty() {
        let text = bytes_to_string(source[found.start..found.end].to_vec());
        return make_lua_string(l, &text).map(|s| vec![Value::String(s)]);
    }

    found
        .captures
        .iter()
        .map(|capture| capture_to_value(l, source, capture))
        .collect()
}

fn gsub_table_key(l: &mut LuaState, source: &[u8], found: &PatternMatch) -> Option<Value> {
    if let Some(capture) = found.captures.first() {
        capture_to_value(l, source, capture)
    } else {
        let text = bytes_to_string(source[found.start..found.end].to_vec());
        make_lua_string(l, &text).map(Value::String)
    }
}

fn gsub_table_replacement_value(
    l: &mut LuaState,
    table_ref: GcRef<Table>,
    key: &Value,
) -> Option<Value> {
    let mut current = Value::Table(table_ref);
    for _ in 0..100 {
        let Value::Table(current_ref) = current.clone() else {
            return Some(Value::Nil);
        };
        // SAFETY: replacement table and any chained __index table are reachable from
        // the active argument table or its metatable while gsub is executing.
        let table = unsafe { current_ref.as_ref() }?;
        let raw_value = table.get(key);
        if !raw_value.is_nil() {
            return Some(raw_value);
        }

        let index = table
            .metatable()
            .and_then(|metatable| lookup_string_metamethod(metatable, "__index"));
        match index {
            Some(Value::Table(next_ref)) => current = Value::Table(next_ref),
            Some(index_func @ Value::Function(_)) => {
                return call_index_metamethod(l, index_func, current, key);
            }
            _ => return Some(Value::Nil),
        }
    }

    push_lua_string(l, "'__index' chain too long");
    None
}

fn lookup_string_metamethod(metatable: GcRef<Table>, name: &str) -> Option<Value> {
    // SAFETY: metatable is held by a reachable table.
    let metatable = unsafe { metatable.as_ref() }?;
    for (key, value) in metatable.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: key is held by the metatable.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
            && key_string.data() == name
            && !value.is_nil()
        {
            return Some(value.clone());
        }
    }
    None
}

fn call_index_metamethod(
    l: &mut LuaState,
    index_func: Value,
    table_value: Value,
    key: &Value,
) -> Option<Value> {
    let Some(gc_ptr) = l.gc else {
        push_lua_string(l, "gsub unavailable without an active GC");
        return None;
    };
    // SAFETY: LuaState::gc is installed by the VM for the duration of execution.
    let gc = unsafe { &mut *gc_ptr };
    match call_value(l, gc, index_func, &[table_value, key.clone()], Some(1)) {
        Ok(results) => Some(results.first().cloned().unwrap_or(Value::Nil)),
        Err(err) => {
            if let Some(error_value) = err.error_value() {
                l.push_value(error_value);
            } else {
                push_lua_string(l, &err.message);
            }
            None
        }
    }
}

fn capture_to_value(l: &mut LuaState, source: &[u8], capture: &Capture) -> Option<Value> {
    match capture {
        Capture::Open(_) => None,
        Capture::Range(start, end) => {
            let text = bytes_to_string(source[*start..*end].to_vec());
            make_lua_string(l, &text).map(Value::String)
        }
        Capture::Position(pos) => Some(Value::Number((*pos + 1) as f64)),
    }
}

fn replacement_value_to_string(
    source: &[u8],
    found: &PatternMatch,
    value: &Value,
) -> Option<String> {
    match value {
        Value::Nil | Value::Boolean(false) => {
            Some(bytes_to_string(source[found.start..found.end].to_vec()))
        }
        Value::String(s) => {
            // SAFETY: table replacement value is reachable from an active table argument.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        Value::Number(n) => Some(number_to_lua_string(*n)),
        _ => None,
    }
}

fn expand_replacement_template(
    template: &[u8],
    source: &[u8],
    found: &PatternMatch,
) -> Result<String, String> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < template.len() {
        if template[idx] == b'%' && idx + 1 < template.len() {
            let escaped = template[idx + 1];
            match escaped {
                b'%' => out.push(b'%'),
                b'0' => out.extend_from_slice(&source[found.start..found.end]),
                b'1'..=b'9' => {
                    if let Some(bytes) =
                        replacement_capture_bytes(source, found, (escaped - b'0') as usize)
                    {
                        out.extend_from_slice(&bytes);
                    } else {
                        return Err("invalid capture index".to_string());
                    }
                }
                _ => out.push(escaped),
            }
            idx += 2;
        } else {
            out.push(template[idx]);
            idx += 1;
        }
    }
    Ok(bytes_to_string(out))
}

fn replacement_capture_bytes(
    source: &[u8],
    found: &PatternMatch,
    capture_idx: usize,
) -> Option<Vec<u8>> {
    if capture_idx == 0 || found.captures.is_empty() {
        return Some(source[found.start..found.end].to_vec());
    }

    match found.captures.get(capture_idx - 1)? {
        Capture::Open(_) => None,
        Capture::Range(start, end) => Some(source[*start..*end].to_vec()),
        Capture::Position(pos) => Some(number_to_lua_string((*pos + 1) as f64).into_bytes()),
    }
}

fn push_captures(l: &mut LuaState, source: &[u8], captures: &[Capture]) -> bool {
    for capture in captures {
        match capture {
            Capture::Open(_) => return false,
            Capture::Range(start, end) => {
                let result = bytes_to_string(source[*start..*end].to_vec());
                if !push_lua_string(l, &result) {
                    return false;
                }
            }
            Capture::Position(pos) => l.push_value(Value::Number((*pos + 1) as f64)),
        }
    }
    true
}

unsafe extern "C" fn lua_string_upper(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(mut s) = string_arg(l, 1) else {
        return -1;
    };
    s.make_ascii_uppercase();
    if push_lua_string(l, &bytes_to_string(s)) {
        1
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_lower(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(mut s) = string_arg(l, 1) else {
        return -1;
    };
    s.make_ascii_lowercase();
    if push_lua_string(l, &bytes_to_string(s)) {
        1
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_reverse(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(mut s) = string_arg(l, 1) else {
        return -1;
    };
    s.reverse();
    if push_lua_string(l, &bytes_to_string(s)) {
        1
    } else {
        -1
    }
}

unsafe extern "C" fn lua_string_rep(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr comes from the VM CALL handler.
    let l = unsafe { to_lua(l_ptr) };
    let Some(s) = string_arg(l, 1) else {
        return -1;
    };
    let Some(n) = number_arg(l, 2) else {
        return -1;
    };
    let count = n as i32;
    let result = if count <= 0 {
        String::new()
    } else {
        bytes_to_string(s).repeat(count as usize)
    };
    if push_lua_string(l, &result) { 1 } else { -1 }
}

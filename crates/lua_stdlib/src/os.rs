//! OS 库 (Operating System Library)
//!

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

static CLOCK_START: OnceLock<Instant> = OnceLock::new();
static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn open_os(l: &mut LuaState, gc: &mut GarbageCollector) {
    let os_table = find_lib_table(l, "os");
    if os_table.is_null() {
        return;
    }

    reg(l, gc, os_table, "clock", lua_os_clock);
    reg(l, gc, os_table, "date", lua_os_date);
    reg(l, gc, os_table, "difftime", lua_os_difftime);
    reg(l, gc, os_table, "execute", lua_os_execute);
    reg(l, gc, os_table, "remove", lua_os_remove);
    reg(l, gc, os_table, "rename", lua_os_rename);
    reg(l, gc, os_table, "setlocale", lua_os_setlocale);
    reg(l, gc, os_table, "time", lua_os_time);
    reg(l, gc, os_table, "tmpname", lua_os_tmpname);
}

fn reg(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), func, None)
        .expect("OS Function publication must remain collector-valid");
}

fn find_lib_table(l: &LuaState, name: &str) -> GcRef<Table> {
    crate::registration::find_library_table(l, name.as_bytes())
        .ok()
        .flatten()
        .unwrap_or_else(GcRef::null)
}

unsafe extern "C" fn lua_os_clock(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let start = CLOCK_START.get_or_init(Instant::now);
    l.push_value(Value::Number(start.elapsed().as_secs_f64()));
    1
}

unsafe extern "C" fn lua_os_time(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as f64)
                .unwrap_or(0.0);
            l.push_value(Value::Number(now));
            1
        }
        Value::Table(table_ref) => {
            // SAFETY: the time table is held on the active Lua stack.
            let Some(table) = (unsafe { table_ref.as_ref() }) else {
                l.push_nil();
                return 1;
            };
            let year = table_number_field(l, table, "year").unwrap_or(1970.0) as i64;
            let month = table_number_field(l, table, "month").unwrap_or(1.0) as i64;
            let day = table_number_field(l, table, "day").unwrap_or(1.0) as i64;
            let hour = table_number_field(l, table, "hour").unwrap_or(12.0) as i64;
            let min = table_number_field(l, table, "min").unwrap_or(0.0) as i64;
            let sec = table_number_field(l, table, "sec").unwrap_or(0.0) as i64;
            let days = days_from_civil(year, month, day);
            l.push_value(Value::Number(
                (days * 86_400 + hour * 3_600 + min * 60 + sec) as f64,
            ));
            1
        }
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_os_difftime(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let a = number_arg(l, 1).unwrap_or(0.0);
    let b = number_arg(l, 2).unwrap_or(0.0);
    l.push_value(Value::Number(a - b));
    1
}

unsafe extern "C" fn lua_os_execute(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let command = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => {
            l.push_value(Value::Number(1.0));
            return 1;
        }
        Value::String(s) => {
            let Some(command) = l
                .with_string_bytes(s, |bytes| {
                    std::str::from_utf8(bytes).ok().map(str::to_owned)
                })
                .ok()
                .flatten()
            else {
                l.push_nil();
                let _ = push_lua_string(l, "os.execute command must be valid UTF-8");
                return 2;
            };
            command
        }
        _ => {
            l.push_nil();
            return 1;
        }
    };

    #[cfg(windows)]
    let status = {
        let mut shell = std::process::Command::new("cmd");
        let raw_command = if command.trim_start().starts_with('"') {
            format!(" \"{command}\"")
        } else {
            format!(" {command}")
        };
        shell.arg("/C").raw_arg(&raw_command).status()
    };

    #[cfg(not(windows))]
    let status = std::process::Command::new("sh")
        .args(["-c", &command])
        .status();

    match status {
        Ok(status) => {
            l.push_value(Value::Number(status.code().unwrap_or(1) as f64));
            1
        }
        Err(err) => {
            l.push_nil();
            let _ = push_lua_string(l, &err.to_string());
            2
        }
    }
}

unsafe extern "C" fn lua_os_date(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let format = match l.at(1) {
        None | Some(Value::Nil) => "%c".to_string(),
        Some(Value::String(string_ref)) => {
            let Some(format) = l
                .with_string_bytes(*string_ref, |bytes| {
                    std::str::from_utf8(bytes).ok().map(str::to_owned)
                })
                .ok()
                .flatten()
            else {
                l.push_nil();
                let _ = push_lua_string(l, "os.date format must be valid UTF-8");
                return 2;
            };
            format
        }
        Some(_) => {
            l.push_nil();
            return 1;
        }
    };
    let timestamp = number_arg(l, 2).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as f64)
            .unwrap_or(0.0)
    }) as i64;
    let format = format.strip_prefix('!').unwrap_or(&format).to_string();
    let parts = DateParts::from_timestamp(timestamp);

    if format == "*t" {
        let Some(gc_ptr) = l.gc else {
            l.push_nil();
            return 1;
        };
        // SAFETY: LuaState::gc is installed by the VM before calling C functions.
        let gc = unsafe { &mut *gc_ptr };
        let publication = gc.with_publication(|transaction| {
            let table = transaction.alloc(Table::new());
            for (name, value) in [
                (b"year".as_slice(), parts.year as f64),
                (b"month".as_slice(), parts.month as f64),
                (b"day".as_slice(), parts.day as f64),
                (b"hour".as_slice(), parts.hour as f64),
                (b"min".as_slice(), parts.min as f64),
                (b"sec".as_slice(), parts.sec as f64),
                (b"wday".as_slice(), (parts.wday + 1) as f64),
                (b"yday".as_slice(), parts.yday as f64),
            ] {
                let key = crate::registration::rooted_bytes(l, transaction, name)?;
                transaction.set_table_value(&table, &key, &Value::Number(value))?;
            }
            let isdst = crate::registration::rooted_bytes(l, transaction, b"isdst")?;
            transaction.set_table_value(&table, &isdst, &Value::Boolean(false))?;
            // SAFETY: the completed result Table is installed on the active
            // stack before its temporary root is released.
            unsafe { transaction.publish_table_value(table, |value| l.push_value(value)) }
        });
        if publication.is_err() {
            l.push_nil();
        }
        return 1;
    }

    push_lua_string(l, &format_date(&format, &parts))
}

unsafe extern "C" fn lua_os_setlocale(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };

    let locale = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => return push_lua_string(l, "C"),
        Value::String(s) => l.copy_string_bytes(s).unwrap_or_default(),
        _ => {
            l.push_nil();
            return 1;
        }
    };

    match locale.as_slice() {
        b"" | b"C" => push_lua_string(l, "C"),
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_os_remove(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(path) = string_arg_text(l, 1) else {
        l.push_nil();
        return 1;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {
            l.push_value(Value::Boolean(true));
            1
        }
        Err(err) => {
            l.push_nil();
            let _ = push_lua_string(l, &err.to_string());
            2
        }
    }
}

unsafe extern "C" fn lua_os_rename(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(from) = string_arg_text(l, 1) else {
        l.push_nil();
        return 1;
    };
    let Some(to) = string_arg_text(l, 2) else {
        l.push_nil();
        return 1;
    };
    match std::fs::rename(&from, &to) {
        Ok(()) => {
            l.push_value(Value::Boolean(true));
            1
        }
        Err(err) => {
            l.push_nil();
            let _ = push_lua_string(l, &err.to_string());
            2
        }
    }
}

unsafe extern "C" fn lua_os_tmpname(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let count = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "lua_rust_tmp_{}_{}_{}",
        std::process::id(),
        stamp,
        count
    ));
    push_lua_string(l, &path.to_string_lossy())
}

fn string_arg_text(l: &LuaState, idx: i32) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // Host OS APIs accept text paths/commands, so this boundary rejects
            // arbitrary Lua bytes instead of silently re-encoding them.
            l.with_string_bytes(*s, |bytes| {
                std::str::from_utf8(bytes).ok().map(str::to_owned)
            })
            .ok()
            .flatten()
        }
        _ => None,
    }
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        Some(Value::String(s)) => l
            .with_string_bytes(*s, parse_lua_number_bytes)
            .ok()
            .flatten(),
        _ => None,
    }
}

fn table_number_field(l: &LuaState, table: &Table, name: &str) -> Option<f64> {
    match table_field(l, table, name) {
        Value::Number(n) => Some(n),
        Value::String(s) => l
            .with_string_bytes(s, parse_lua_number_bytes)
            .ok()
            .flatten(),
        _ => None,
    }
}

fn table_field(l: &LuaState, table: &Table, name: &str) -> Value {
    let Ok(Some(key)) = l.find_interned_string(name.as_bytes()) else {
        return Value::Nil;
    };
    table.get(&Value::String(key))
}

struct DateParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    min: i64,
    sec: i64,
    wday: i64,
    yday: i64,
}

impl DateParts {
    fn from_timestamp(timestamp: i64) -> Self {
        let days = timestamp.div_euclid(86_400);
        let seconds = timestamp.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        let yday = days - days_from_civil(year, 1, 1) + 1;
        Self {
            year,
            month,
            day,
            hour: seconds / 3_600,
            min: (seconds % 3_600) / 60,
            sec: seconds % 60,
            wday: (days + 4).rem_euclid(7),
            yday,
        }
    }
}

fn format_date(format: &str, parts: &DateParts) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{:04}", parts.year)),
            Some('m') => out.push_str(&format!("{:02}", parts.month)),
            Some('d') => out.push_str(&format!("{:02}", parts.day)),
            Some('H') => out.push_str(&format!("{:02}", parts.hour)),
            Some('M') => out.push_str(&format!("{:02}", parts.min)),
            Some('S') => out.push_str(&format!("{:02}", parts.sec)),
            Some('w') => out.push_str(&parts.wday.to_string()),
            Some('j') => out.push_str(&format!("{:03}", parts.yday)),
            Some('%') => out.push('%'),
            Some('c') => out.push_str(&format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                parts.year, parts.month, parts.day, parts.hour, parts.min, parts.sec
            )),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_adj = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_adj + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_lua_number_bytes(mut bytes: &[u8]) -> Option<f64> {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    std::str::from_utf8(bytes).ok()?.parse::<f64>().ok()
}

fn push_lua_string(l: &mut LuaState, text: &str) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    if crate::registration::push_string(l, gc, text.as_bytes()).is_err() {
        l.push_nil();
    }
    1
}

#[cfg(test)]
mod byte_string_tests {
    use super::*;

    #[test]
    fn number_parser_rejects_non_text_lua_bytes() {
        assert_eq!(parse_lua_number_bytes(b" 123.5 "), Some(123.5));
        assert_eq!(parse_lua_number_bytes(&[b'1', 0xff]), None);
        assert_eq!(parse_lua_number_bytes(b"12\0"), None);
    }
}

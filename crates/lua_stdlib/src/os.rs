//! OS 库 (Operating System Library)
//!

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
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

    let table_ptr = os_table.as_ptr() as *mut Table;
    reg(gc, table_ptr, "clock", lua_os_clock);
    reg(gc, table_ptr, "date", lua_os_date);
    reg(gc, table_ptr, "difftime", lua_os_difftime);
    reg(gc, table_ptr, "execute", lua_os_execute);
    reg(gc, table_ptr, "remove", lua_os_remove);
    reg(gc, table_ptr, "rename", lua_os_rename);
    reg(gc, table_ptr, "setlocale", lua_os_setlocale);
    reg(gc, table_ptr, "time", lua_os_time);
    reg(gc, table_ptr, "tmpname", lua_os_tmpname);
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
            let year = table_number_field(table, "year").unwrap_or(1970.0) as i64;
            let month = table_number_field(table, "month").unwrap_or(1.0) as i64;
            let day = table_number_field(table, "day").unwrap_or(1.0) as i64;
            let hour = table_number_field(table, "hour").unwrap_or(12.0) as i64;
            let min = table_number_field(table, "min").unwrap_or(0.0) as i64;
            let sec = table_number_field(table, "sec").unwrap_or(0.0) as i64;
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
            // SAFETY: command string is held by the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| s.data().to_string())
                .unwrap_or_default()
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
    let format = string_arg(l, 1).unwrap_or_else(|| "%c".to_string());
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
        let mut table = Table::new();
        set_number_field(&mut table, gc, "year", parts.year as f64);
        set_number_field(&mut table, gc, "month", parts.month as f64);
        set_number_field(&mut table, gc, "day", parts.day as f64);
        set_number_field(&mut table, gc, "hour", parts.hour as f64);
        set_number_field(&mut table, gc, "min", parts.min as f64);
        set_number_field(&mut table, gc, "sec", parts.sec as f64);
        set_number_field(&mut table, gc, "wday", (parts.wday + 1) as f64);
        set_number_field(&mut table, gc, "yday", parts.yday as f64);
        set_bool_field(&mut table, gc, "isdst", false);
        let table_ref = gc.create(table);
        l.push_value(Value::Table(table_ref));
        return 1;
    }

    push_lua_string(l, &format_date(&format, &parts))
}

unsafe extern "C" fn lua_os_setlocale(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };

    let locale = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => return push_lua_string(l, "C"),
        Value::String(s) => {
            // SAFETY: argument strings are kept alive on the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| s.data().to_string())
                .unwrap_or_default()
        }
        _ => {
            l.push_nil();
            return 1;
        }
    };

    match locale.as_str() {
        "" | "C" => push_lua_string(l, "C"),
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_os_remove(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(path) = string_arg(l, 1) else {
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
    let Some(from) = string_arg(l, 1) else {
        l.push_nil();
        return 1;
    };
    let Some(to) = string_arg(l, 2) else {
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

fn string_arg(l: &LuaState, idx: i32) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: string arguments are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        _ => None,
    }
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        Some(Value::String(s)) => {
            // SAFETY: string arguments are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.and_then(|s| s.data().trim().parse::<f64>().ok())
        }
        _ => None,
    }
}

fn table_number_field(table: &Table, name: &str) -> Option<f64> {
    match table_field(table, name) {
        Value::Number(n) => Some(n),
        Value::String(s) => {
            // SAFETY: string value is owned by this live table.
            unsafe { s.as_ref() }.and_then(|s| s.data().trim().parse::<f64>().ok())
        }
        _ => None,
    }
}

fn table_field(table: &Table, name: &str) -> Value {
    for (key, value) in table.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: keys are owned by this live table.
            && let Some(key_str) = unsafe { key_ref.as_ref() }
            && key_str.data() == name
        {
            return value.clone();
        }
    }
    Value::Nil
}

fn set_number_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: f64) {
    let key = gc.create(GcString::new(name));
    table.set(&Value::String(key), &Value::Number(value));
}

fn set_bool_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: bool) {
    let key = gc.create(GcString::new(name));
    table.set(&Value::String(key), &Value::Boolean(value));
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

fn push_lua_string(l: &mut LuaState, text: &str) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let s = gc.create(GcString::new(text));
    l.push_value(Value::String(s));
    1
}

//! IO 库 (Input/Output Library)
//!
//! 当前实现提供 Lua 5.1 `io.tmpfile()` 所需的内存文件对象，覆盖
//! 官方 `math.lua` 生成临时代码再 `loadstring` 的工作流。

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::userdata::Userdata;
use lua_core::value::Value;
use lua_vm::state::LuaState;

use std::io::{Seek, SeekFrom, Write};

const DIRECT_WRITE_THRESHOLD: usize = 256 * 1024;

struct IoFileData {
    direct_handle: Option<std::fs::File>,
}

pub fn open_io(l: &mut LuaState, gc: &mut GarbageCollector) {
    let io_table = find_lib_table(l, "io");
    if io_table.is_null() {
        return;
    }

    let table_ptr = io_table.as_ptr() as *mut Table;
    let stdout = create_memory_file(gc, None, "w", true);
    let stdin = create_memory_file(gc, None, "r", false);
    let stderr = create_memory_file(gc, None, "w", true);
    set_table_value(table_ptr, gc, "stdout", &Value::Userdata(stdout));
    set_table_value(table_ptr, gc, "stdin", &Value::Userdata(stdin));
    set_table_value(table_ptr, gc, "stderr", &Value::Userdata(stderr));
    set_table_value(table_ptr, gc, "__output", &Value::Userdata(stdout));
    set_table_value(table_ptr, gc, "__input", &Value::Userdata(stdin));
    reg(gc, table_ptr, "close", lua_io_close);
    reg(gc, table_ptr, "flush", lua_io_flush);
    reg(gc, table_ptr, "input", lua_io_input);
    reg(gc, table_ptr, "lines", lua_io_lines);
    reg(gc, table_ptr, "open", lua_io_open);
    reg(gc, table_ptr, "output", lua_io_output);
    reg(gc, table_ptr, "read", lua_io_read);
    reg(gc, table_ptr, "tmpfile", lua_io_tmpfile);
    reg(gc, table_ptr, "type", lua_io_type);
    reg(gc, table_ptr, "write", lua_io_write);
}

fn reg(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: table points to a library or file-handle table kept alive by GC roots/stack.
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

fn create_memory_file(
    gc: &mut GarbageCollector,
    path: Option<String>,
    mode: &str,
    writable: bool,
) -> GcRef<Userdata> {
    let mut state = Table::new();
    let initial = if mode.contains('a') || (mode.starts_with('r') && !mode.starts_with('w')) {
        path.as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .map(bytes_to_string)
            .unwrap_or_default()
    } else {
        String::new()
    };
    let initial_len = initial.chars().count();
    set_string_field(&mut state, gc, "__content", &initial);
    set_number_field(
        &mut state,
        gc,
        "__pos",
        if mode.contains('a') {
            initial_len as f64
        } else {
            0.0
        },
    );
    set_bool_field(&mut state, gc, "__closed", false);
    set_bool_field(&mut state, gc, "__writable", writable);
    set_bool_field(
        &mut state,
        gc,
        "__readable",
        mode.starts_with('r') || mode.contains('+'),
    );
    set_string_field(&mut state, gc, "__mode", mode);
    set_string_field(&mut state, gc, "__buffer", "full");
    set_bool_field(&mut state, gc, "__buffer_explicit", false);
    set_bool_field(&mut state, gc, "__direct", false);
    if let Some(path) = path {
        set_string_field(&mut state, gc, "__path", &path);
    }

    let file_ptr = &mut state as *mut Table;
    reg(gc, file_ptr, "write", lua_io_file_write);
    reg(gc, file_ptr, "read", lua_io_file_read);
    reg(gc, file_ptr, "seek", lua_io_file_seek);
    reg(gc, file_ptr, "close", lua_io_file_close);
    reg(gc, file_ptr, "setvbuf", lua_io_file_setvbuf);
    reg(gc, file_ptr, "lines", lua_io_file_lines);
    reg(gc, file_ptr, "flush", lua_io_file_flush);
    reg(gc, file_ptr, "__gc", lua_io_file_gc);
    reg(gc, file_ptr, "__tostring", lua_io_file_tostring);
    let index_key = gc.create(GcString::new("__index"));
    let state_ref = gc.create(state);
    // SAFETY: state_ref points to the freshly allocated metatable.
    unsafe {
        let state_table = &mut *(state_ref.as_ptr() as *mut Table);
        state_table.set(&Value::String(index_key), &Value::Table(state_ref));
    }

    let mut userdata = Userdata::new(std::mem::size_of::<IoFileData>());
    // SAFETY: the userdata was allocated with enough space for IoFileData and
    // has no constructed payload yet.
    unsafe {
        userdata.write_typed(IoFileData {
            direct_handle: None,
        });
    }
    userdata.set_metatable(Some(state_ref));
    gc.create(userdata)
}

fn set_table_value(table: *mut Table, gc: &mut GarbageCollector, key: &str, value: &Value) {
    let key = gc.create(GcString::new(key));
    // SAFETY: table points to a live library/file table during registration.
    unsafe {
        (*table).set(&Value::String(key), value);
    }
}

fn set_table_ref_string(
    table_ref: GcRef<Table>,
    gc: &mut GarbageCollector,
    key: &str,
    value: &Value,
) {
    if table_ref.is_null() {
        return;
    }
    let key = gc.create(GcString::new(key));
    // SAFETY: table_ref is reachable from globals while IO functions execute.
    unsafe {
        let table = &mut *(table_ref.as_ptr() as *mut Table);
        table.set(&Value::String(key), value);
    }
}

fn table_get_string(table_ref: GcRef<Table>, key: &str) -> Value {
    // SAFETY: table_ref is reachable from globals while IO functions execute.
    let Some(table) = (unsafe { table_ref.as_ref() }) else {
        return Value::Nil;
    };
    get_field(table, key)
}

fn current_output(l: &LuaState) -> Option<GcRef<Userdata>> {
    let io_table = find_lib_table(l, "io");
    match table_get_string(io_table, "__output") {
        Value::Userdata(file_ref) => Some(file_ref),
        Value::Table(file_ref) => table_to_file_userdata(file_ref),
        _ => None,
    }
}

fn current_input(l: &LuaState) -> Option<GcRef<Userdata>> {
    let io_table = find_lib_table(l, "io");
    match table_get_string(io_table, "__input") {
        Value::Userdata(file_ref) => Some(file_ref),
        Value::Table(file_ref) => table_to_file_userdata(file_ref),
        _ => None,
    }
}

fn set_current_output(
    gc: &mut GarbageCollector,
    io_table: GcRef<Table>,
    file_ref: GcRef<Userdata>,
) {
    if let Value::Userdata(previous) = table_get_string(io_table, "__output")
        && previous != file_ref
    {
        let _ = flush_file_to_disk(previous);
    }
    set_table_ref_string(io_table, gc, "__output", &Value::Userdata(file_ref));
}

fn table_to_file_userdata(_table_ref: GcRef<Table>) -> Option<GcRef<Userdata>> {
    None
}

fn file_state(file_ref: GcRef<Userdata>) -> Option<&'static Table> {
    // SAFETY: file_ref is held by the active stack/table/function environment while
    // stdlib code is executing, and the userdata metatable stores the file state.
    let userdata = unsafe { file_ref.as_ref() }?;
    let metatable = userdata.metatable()?;
    if metatable.is_null() {
        None
    } else {
        // SAFETY: the metatable is kept alive by the userdata.
        Some(unsafe { &*metatable.as_ptr() })
    }
}

fn file_state_mut(file_ref: GcRef<Userdata>) -> Option<&'static mut Table> {
    // SAFETY: file_ref is held by the active stack/table/function environment while
    // stdlib code is executing. The VM is single-threaded, so this mutable access is
    // scoped to the current C-library operation.
    let userdata = unsafe { file_ref.as_ref() }?;
    let metatable = userdata.metatable()?;
    // SAFETY: the metatable is kept alive by the userdata and stores mutable state.
    unsafe { (metatable.as_ptr() as *mut Table).as_mut() }
}

fn open_file_handle(
    gc: &mut GarbageCollector,
    path: &str,
    mode: &str,
) -> std::io::Result<GcRef<Userdata>> {
    let read_mode = mode.starts_with('r');
    let append_mode = mode.starts_with('a');
    let write_mode = mode.starts_with('w') || append_mode || mode.contains('+');
    let binary_mode = mode.contains('b');
    let normalized_mode = if binary_mode {
        mode.replace('b', "")
    } else {
        mode.to_string()
    };

    if read_mode && !std::path::Path::new(path).is_file() {
        std::fs::File::open(path)?;
    }
    if write_mode {
        if let Some(parent) = std::path::Path::new(path).parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true);
        if normalized_mode.starts_with('w') {
            options.truncate(true);
        }
        if append_mode {
            options.append(true);
        }
        if normalized_mode.contains('+') {
            options.read(true);
        }
        options.open(path)?;
    }

    Ok(create_memory_file(
        gc,
        Some(path.to_string()),
        &normalized_mode,
        write_mode,
    ))
}

unsafe extern "C" fn lua_io_tmpfile(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let file_ref = create_memory_file(gc, None, "w+", true);
    l.push_value(Value::Userdata(file_ref));
    1
}

unsafe extern "C" fn lua_io_input(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let io_table = find_lib_table(l, "io");
    if io_table.is_null() {
        l.push_nil();
        return 1;
    }

    match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => {
            l.push_value(table_get_string(io_table, "__input"));
            1
        }
        Value::String(path_ref) => {
            // SAFETY: path argument is on the active stack.
            let path = unsafe { path_ref.as_ref() }
                .map(|path| path.data().to_string())
                .unwrap_or_default();
            match open_file_handle(gc, &path, "r") {
                Ok(file_ref) => {
                    set_table_ref_string(io_table, gc, "__input", &Value::Userdata(file_ref));
                    l.push_value(Value::Userdata(file_ref));
                    1
                }
                Err(err) => {
                    l.push_nil();
                    push_lua_string(l, gc, &err.to_string());
                    l.push_value(Value::Number(err.raw_os_error().unwrap_or(0) as f64));
                    3
                }
            }
        }
        Value::Userdata(file_ref) => {
            set_table_ref_string(io_table, gc, "__input", &Value::Userdata(file_ref));
            l.push_value(Value::Userdata(file_ref));
            1
        }
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_io_open(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(path) = string_arg(l, 1) else {
        l.push_nil();
        return 1;
    };
    let mode = string_arg(l, 2).unwrap_or_else(|| "r".to_string());
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    match open_file_handle(gc, &path, &mode) {
        Ok(file_ref) => {
            l.push_value(Value::Userdata(file_ref));
            1
        }
        Err(err) => {
            l.push_nil();
            push_lua_string(l, gc, &err.to_string());
            l.push_value(Value::Number(err.raw_os_error().unwrap_or(0) as f64));
            3
        }
    }
}

unsafe extern "C" fn lua_io_read(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = current_input(l) else {
        l.push_nil();
        return 1;
    };
    read_from_file(l, file_ref, 1)
}

unsafe extern "C" fn lua_io_type(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Userdata(file_ref) => {
            let Some(state) = file_state(file_ref) else {
                l.push_nil();
                return 1;
            };
            if get_bool_field(state, "__closed") {
                push_lua_string(l, gc, "closed file");
            } else {
                push_lua_string(l, gc, "file");
            }
            1
        }
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_io_flush(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = current_output(l) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    l.push_value(Value::Boolean(flush_file_to_disk(file_ref).is_ok()));
    1
}

unsafe extern "C" fn lua_io_lines(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => {
            let Some(file_ref) = current_input(l) else {
                l.push_nil();
                return 1;
            };
            push_lines_iterator(l, gc, file_ref, false)
        }
        Value::String(path_ref) => {
            // SAFETY: path argument is on the active stack.
            let path = unsafe { path_ref.as_ref() }
                .map(|path| path.data().to_string())
                .unwrap_or_default();
            match open_file_handle(gc, &path, "r") {
                Ok(file_ref) => push_lines_iterator(l, gc, file_ref, true),
                Err(err) => {
                    l.push_nil();
                    push_lua_string(l, gc, &err.to_string());
                    l.push_value(Value::Number(err.raw_os_error().unwrap_or(0) as f64));
                    3
                }
            }
        }
        Value::Userdata(file_ref) => push_lines_iterator(l, gc, file_ref, false),
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_io_output(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let io_table = find_lib_table(l, "io");
    if io_table.is_null() {
        l.push_nil();
        return 1;
    }

    match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => {
            l.push_value(table_get_string(io_table, "__output"));
            1
        }
        Value::String(path_ref) => {
            // SAFETY: path argument is on the active stack.
            let path = unsafe { path_ref.as_ref() }
                .map(|path| path.data().to_string())
                .unwrap_or_default();
            match open_file_handle(gc, &path, "w") {
                Ok(file_ref) => {
                    set_current_output(gc, io_table, file_ref);
                    l.push_value(Value::Userdata(file_ref));
                    1
                }
                Err(err) => {
                    l.push_nil();
                    push_lua_string(l, gc, &err.to_string());
                    l.push_value(Value::Number(err.raw_os_error().unwrap_or(0) as f64));
                    3
                }
            }
        }
        Value::Userdata(file_ref) => {
            set_current_output(gc, io_table, file_ref);
            l.push_value(Value::Userdata(file_ref));
            1
        }
        _ => {
            l.push_nil();
            1
        }
    }
}

unsafe extern "C" fn lua_io_write(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = current_output(l) else {
        l.push_nil();
        return 1;
    };
    write_to_file(l, file_ref, 1, true)
}

unsafe extern "C" fn lua_io_close(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let file_ref = match l.at(1).cloned().unwrap_or(Value::Nil) {
        Value::Nil => current_output(l),
        Value::Userdata(file_ref) => Some(file_ref),
        _ => None,
    };
    let Some(file_ref) = file_ref else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    close_file_handle(l, file_ref)
}

unsafe extern "C" fn lua_io_file_write(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_nil();
        return 1;
    };
    write_to_file(l, file_ref, 2, false)
}

fn write_to_file(
    l: &mut LuaState,
    file_ref: GcRef<Userdata>,
    first_arg: i32,
    throw_on_error: bool,
) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let mut appended = String::new();
    for idx in first_arg..=l.get_top() {
        let value = l.at(idx).cloned().unwrap_or(Value::Nil);
        appended.push_str(&value_to_write_string(&value));
    }

    let Some(file) = file_state_mut(file_ref) else {
        l.push_nil();
        return 1;
    };

    if get_bool_field(file, "__closed") {
        if throw_on_error {
            return push_error(l, gc, "attempt to use a closed file");
        }
        l.push_nil();
        push_lua_string(l, gc, "file is closed");
        l.push_value(Value::Number(0.0));
        return 3;
    }

    if !get_bool_field(file, "__writable") {
        l.push_nil();
        push_lua_string(l, gc, "file is not open for writing");
        l.push_value(Value::Number(0.0));
        return 3;
    }

    let pos = get_number_field(file, "__pos").max(0.0) as usize;
    let already_direct = get_bool_field(file, "__direct");
    let content = if already_direct {
        String::new()
    } else {
        get_string_field(file, "__content")
    };
    if should_write_direct(file, already_direct, &content, pos, &appended) {
        match write_direct(gc, file_ref, file, &content, pos, &appended) {
            Ok(new_pos) => {
                set_number_field(file, gc, "__pos", new_pos as f64);
                l.push_value(Value::Userdata(file_ref));
                return 1;
            }
            Err(err) => {
                l.push_nil();
                push_lua_string(l, gc, &err.to_string());
                l.push_value(Value::Number(0.0));
                return 3;
            }
        }
    }

    let new_content = write_at(&content, pos, &appended);
    let new_pos = pos + appended.chars().count();
    set_string_field(file, gc, "__content", &new_content);
    set_number_field(file, gc, "__pos", new_pos as f64);

    let buffer_mode = get_string_field(file, "__buffer");
    if buffer_mode == "no" || (buffer_mode == "line" && appended.contains('\n')) {
        let _ = flush_file_to_disk(file_ref);
    }

    l.push_value(Value::Userdata(file_ref));
    1
}

unsafe extern "C" fn lua_io_file_read(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_nil();
        return 1;
    };
    read_from_file(l, file_ref, 2)
}

unsafe extern "C" fn lua_io_file_seek(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_nil();
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let whence = string_arg(l, 2).unwrap_or_else(|| "cur".to_string());
    let offset = number_arg(l, 3).unwrap_or(0.0) as isize;
    let Some(file) = file_state_mut(file_ref) else {
        l.push_nil();
        return 1;
    };

    let content_len = if get_bool_field(file, "__direct") {
        direct_file_len(file_ref)
            .unwrap_or_else(|| get_string_field(file, "__content").chars().count()) as isize
    } else {
        get_string_field(file, "__content").chars().count() as isize
    };
    let current = get_number_field(file, "__pos") as isize;
    let base = match whence.as_str() {
        "set" => 0,
        "end" => content_len,
        _ => current,
    };
    let new_pos = (base + offset).clamp(0, content_len) as usize;
    set_number_field(file, gc, "__pos", new_pos as f64);
    l.push_value(Value::Number(new_pos as f64));
    1
}

unsafe extern "C" fn lua_io_file_close(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    close_file_handle(l, file_ref)
}

unsafe extern "C" fn lua_io_file_lines(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_nil();
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    push_lines_iterator(l, gc, file_ref, false)
}

unsafe extern "C" fn lua_io_file_flush(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    l.push_value(Value::Boolean(flush_file_to_disk(file_ref).is_ok()));
    1
}

unsafe extern "C" fn lua_io_file_setvbuf(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(file_ref) = file_arg(l) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    let Some(mode) = string_arg(l, 2) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let Some(file) = file_state_mut(file_ref) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    match mode.as_str() {
        "no" | "full" | "line" => {
            set_string_field(file, gc, "__buffer", &mode);
            set_bool_field(file, gc, "__buffer_explicit", true);
            l.push_value(Value::Boolean(true));
            1
        }
        _ => {
            l.push_value(Value::Boolean(false));
            1
        }
    }
}

unsafe extern "C" fn lua_io_file_tostring(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let text = match file_arg(l).and_then(file_state) {
        Some(file) if get_bool_field(file, "__closed") => "file (closed)".to_string(),
        Some(_) => "file".to_string(),
        None => "file (closed)".to_string(),
    };
    push_lua_string(l, gc, &text);
    1
}

unsafe extern "C" fn lua_io_file_gc(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        return 0;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    if let Some(file_ref) = file_arg(l) {
        let _ = close_file_silent(gc, file_ref);
    } else {
        return push_error(l, gc, "no value");
    }
    0
}

unsafe extern "C" fn lua_io_lines_iter(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let Some(env_ref) = current_c_function_env(l) else {
        l.push_nil();
        return 1;
    };
    // SAFETY: the iterator function owns env_ref through its function environment.
    let Some(env) = (unsafe { env_ref.as_ref() }) else {
        l.push_nil();
        return 1;
    };
    if get_bool_field(env, "__dead") {
        return push_error(l, gc, "file iterator is closed");
    }
    let file_ref = match get_field(env, "__file") {
        Value::Userdata(file_ref) => file_ref,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let auto_close = get_bool_field(env, "__auto_close");

    match read_line_from_file(l, gc, file_ref) {
        Ok(Some(line)) => {
            push_lua_string(l, gc, &line);
            1
        }
        Ok(None) => {
            if auto_close {
                let _ = close_file_silent(gc, file_ref);
            }
            // SAFETY: env_ref is the current iterator's private environment table.
            let env = unsafe { &mut *(env_ref.as_ptr() as *mut Table) };
            set_bool_field(env, gc, "__dead", true);
            0
        }
        Err(message) => push_error(l, gc, &message),
    }
}

fn close_file_handle(l: &mut LuaState, file_ref: GcRef<Userdata>) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let Some(file) = file_state_mut(file_ref) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };

    if get_bool_field(file, "__closed") {
        return push_error(l, gc, "attempt to close a closed file");
    }

    if flush_file_to_disk(file_ref).is_err() {
        l.push_value(Value::Boolean(false));
        return 1;
    }

    close_direct_handle(file_ref);
    set_bool_field(file, gc, "__closed", true);
    l.push_value(Value::Boolean(true));
    1
}

fn close_file_silent(gc: &mut GarbageCollector, file_ref: GcRef<Userdata>) -> Result<(), String> {
    let Some(file) = file_state_mut(file_ref) else {
        return Err("invalid file".to_string());
    };
    if get_bool_field(file, "__closed") {
        return Err("attempt to close a closed file".to_string());
    }
    flush_file_to_disk(file_ref).map_err(|err| err.to_string())?;
    close_direct_handle(file_ref);
    set_bool_field(file, gc, "__closed", true);
    Ok(())
}

#[derive(Debug)]
enum ReadFormat {
    Line,
    All,
    Number,
    Bytes(usize),
}

enum ReadValue {
    Nil,
    String(String),
    Number(f64),
}

fn read_from_file(l: &mut LuaState, file_ref: GcRef<Userdata>, first_arg: i32) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    if let Err(message) = ensure_file_readable(gc, file_ref) {
        return push_error(l, gc, &message);
    }

    let formats = match read_formats_from_args(l, first_arg) {
        Ok(formats) => formats,
        Err(message) => return push_error(l, gc, &message),
    };

    let Some(file) = file_state(file_ref) else {
        l.push_nil();
        return 1;
    };
    if !get_bool_field(file, "__readable") {
        l.push_nil();
        return 1;
    }

    let content = get_string_field(file, "__content");
    let mut pos = get_number_field(file, "__pos").max(0.0) as usize;
    let mut values = Vec::new();
    for format in formats {
        let value = read_one(&content, pos, &format);
        pos = value.1;
        let result = value.0;
        let stop = matches!(result, ReadValue::Nil);
        values.push(result);
        if stop {
            break;
        }
    }

    if let Some(file) = file_state_mut(file_ref) {
        set_number_field(file, gc, "__pos", pos as f64);
    }

    let count = values.len();
    for value in values {
        match value {
            ReadValue::Nil => l.push_nil(),
            ReadValue::String(text) => push_lua_string(l, gc, &text),
            ReadValue::Number(number) => l.push_value(Value::Number(number)),
        }
    }
    count as i32
}

fn read_line_from_file(
    _l: &mut LuaState,
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
) -> Result<Option<String>, String> {
    ensure_file_readable(gc, file_ref)?;
    let Some(file) = file_state(file_ref) else {
        return Err("invalid file".to_string());
    };
    if !get_bool_field(file, "__readable") {
        return Ok(None);
    }
    let content = get_string_field(file, "__content");
    let pos = get_number_field(file, "__pos").max(0.0) as usize;
    match read_one(&content, pos, &ReadFormat::Line) {
        (ReadValue::String(line), new_pos) => {
            if let Some(file) = file_state_mut(file_ref) {
                set_number_field(file, gc, "__pos", new_pos as f64);
            }
            Ok(Some(line))
        }
        (ReadValue::Nil, _) => Ok(None),
        _ => Ok(None),
    }
}

fn ensure_file_readable(
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
) -> Result<(), String> {
    let Some(file) = file_state(file_ref) else {
        return Err("invalid file".to_string());
    };
    if get_bool_field(file, "__closed") {
        return Err("attempt to use a closed file".to_string());
    }
    refresh_file_from_disk(gc, file_ref);
    Ok(())
}

fn read_formats_from_args(l: &LuaState, first_arg: i32) -> Result<Vec<ReadFormat>, String> {
    if l.get_top() < first_arg {
        return Ok(vec![ReadFormat::Line]);
    }

    let mut formats = Vec::new();
    for idx in first_arg..=l.get_top() {
        match l.at(idx).cloned().unwrap_or(Value::Nil) {
            Value::Number(n) if n >= 0.0 => formats.push(ReadFormat::Bytes(n as usize)),
            Value::String(s) => {
                // SAFETY: argument strings are kept alive on the active Lua stack.
                let text = unsafe { s.as_ref() }
                    .map(|s| s.data().to_string())
                    .unwrap_or_default();
                match text.as_str() {
                    "*l" | "*line" => formats.push(ReadFormat::Line),
                    "*a" | "*all" => formats.push(ReadFormat::All),
                    "*n" | "*number" => formats.push(ReadFormat::Number),
                    _ => return Err("invalid read option".to_string()),
                }
            }
            _ => return Err("invalid read option".to_string()),
        }
    }
    Ok(formats)
}

fn read_one(content: &str, pos: usize, format: &ReadFormat) -> (ReadValue, usize) {
    let chars: Vec<char> = content.chars().collect();
    let pos = pos.min(chars.len());
    match format {
        ReadFormat::Line => read_line_chars(&chars, pos),
        ReadFormat::All => {
            let text: String = chars[pos..].iter().collect();
            (ReadValue::String(text), chars.len())
        }
        ReadFormat::Number => read_number_chars(&chars, pos),
        ReadFormat::Bytes(count) => {
            if *count == 0 {
                if pos < chars.len() {
                    (ReadValue::String(String::new()), pos)
                } else {
                    (ReadValue::Nil, pos)
                }
            } else if pos >= chars.len() {
                (ReadValue::Nil, pos)
            } else {
                let end = (pos + *count).min(chars.len());
                let text: String = chars[pos..end].iter().collect();
                (ReadValue::String(text), end)
            }
        }
    }
}

fn read_line_chars(chars: &[char], pos: usize) -> (ReadValue, usize) {
    if pos >= chars.len() {
        return (ReadValue::Nil, pos);
    }
    let mut end = pos;
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
    }
    let mut line_end = end;
    if line_end > pos && chars[line_end - 1] == '\r' {
        line_end -= 1;
    }
    let text: String = chars[pos..line_end].iter().collect();
    let new_pos = if end < chars.len() { end + 1 } else { end };
    (ReadValue::String(text), new_pos)
}

fn read_number_chars(chars: &[char], pos: usize) -> (ReadValue, usize) {
    let mut idx = pos;
    while idx < chars.len() && chars[idx].is_whitespace() {
        idx += 1;
    }
    let start = idx;
    if idx < chars.len() && matches!(chars[idx], '+' | '-') {
        idx += 1;
    }

    let mut digits_before_dot = 0;
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        digits_before_dot += 1;
        idx += 1;
    }

    let mut digits_after_dot = 0;
    if idx < chars.len() && chars[idx] == '.' {
        idx += 1;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            digits_after_dot += 1;
            idx += 1;
        }
    }

    if digits_before_dot == 0 && digits_after_dot == 0 {
        return (ReadValue::Nil, pos);
    }

    let mantissa_end = idx;
    if idx < chars.len() && matches!(chars[idx], 'e' | 'E') {
        let exp_start = idx;
        idx += 1;
        if idx < chars.len() && matches!(chars[idx], '+' | '-') {
            idx += 1;
        }
        let exp_digits_start = idx;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if exp_digits_start == idx {
            idx = exp_start;
        }
    }

    let token_end = idx.max(mantissa_end);
    let token: String = chars[start..token_end].iter().collect();
    match token.parse::<f64>() {
        Ok(number) => (ReadValue::Number(number), token_end),
        Err(_) => (ReadValue::Nil, pos),
    }
}

fn refresh_file_from_disk(gc: &mut GarbageCollector, file_ref: GcRef<Userdata>) {
    let Some(file) = file_state_mut(file_ref) else {
        return;
    };
    if get_bool_field(file, "__writable") {
        return;
    }
    let path = get_string_field(file, "__path");
    if path.is_empty() {
        return;
    }
    if let Ok(bytes) = std::fs::read(&path) {
        let old_pos = get_number_field(file, "__pos").max(0.0) as usize;
        let content = bytes_to_string(bytes);
        let len = content.chars().count();
        set_string_field(file, gc, "__content", &content);
        set_number_field(file, gc, "__pos", old_pos.min(len) as f64);
    }
}

fn flush_file_to_disk(file_ref: GcRef<Userdata>) -> std::io::Result<()> {
    let Some(file) = file_state(file_ref) else {
        return Ok(());
    };
    if get_bool_field(file, "__closed") || !get_bool_field(file, "__writable") {
        return Ok(());
    }
    if get_bool_field(file, "__direct") {
        if let Some(data) = file_data_mut(file_ref)
            && let Some(handle) = data.direct_handle.as_mut()
        {
            handle.flush()?;
        }
        return Ok(());
    }
    let path = get_string_field(file, "__path");
    if path.is_empty() {
        return Ok(());
    }
    let content = get_string_field(file, "__content");
    std::fs::write(path, lua_bytes_from_str(&content))
}

fn should_write_direct(
    file: &Table,
    already_direct: bool,
    content: &str,
    pos: usize,
    appended: &str,
) -> bool {
    if get_bool_field(file, "__buffer_explicit") || get_bool_field(file, "__closed") {
        return false;
    }
    if !get_bool_field(file, "__writable") {
        return false;
    }
    let path = get_string_field(file, "__path");
    if path.is_empty() {
        return false;
    }
    if !get_bool_field(file, "__readable") {
        return true;
    }
    already_direct
        || content.len().saturating_add(appended.len()) > DIRECT_WRITE_THRESHOLD
        || pos > DIRECT_WRITE_THRESHOLD
}

fn write_direct(
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
    file: &mut Table,
    content: &str,
    pos: usize,
    appended: &str,
) -> std::io::Result<usize> {
    let path = get_string_field(file, "__path");
    if !get_bool_field(file, "__direct") {
        std::fs::write(&path, lua_bytes_from_str(content))?;
        set_bool_field(file, gc, "__direct", true);
    }

    if let Some(data) = file_data_mut(file_ref) {
        if data.direct_handle.is_none() {
            data.direct_handle = Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .read(true)
                    .open(&path)?,
            );
        }
        let handle = data.direct_handle.as_mut().expect("direct handle was set");
        handle.seek(SeekFrom::Start(pos as u64))?;
        handle.write_all(&lua_bytes_from_str(appended))?;
    } else {
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&path)?;
        handle.seek(SeekFrom::Start(pos as u64))?;
        handle.write_all(&lua_bytes_from_str(appended))?;
    }
    Ok(pos + appended.chars().count())
}

fn file_data_mut(file_ref: GcRef<Userdata>) -> Option<&'static mut IoFileData> {
    // SAFETY: file_ref points to a live full userdata while stdlib code is running.
    let userdata = unsafe { (file_ref.as_ptr() as *mut Userdata).as_mut() }?;
    // SAFETY: create_memory_file constructs every file userdata with IoFileData.
    unsafe { userdata.data_as_mut::<IoFileData>() }
}

fn close_direct_handle(file_ref: GcRef<Userdata>) {
    if let Some(data) = file_data_mut(file_ref) {
        data.direct_handle.take();
    }
}

fn direct_file_len(file_ref: GcRef<Userdata>) -> Option<usize> {
    if let Some(data) = file_data_mut(file_ref)
        && let Some(handle) = data.direct_handle.as_mut()
    {
        let current = handle.stream_position().ok()?;
        let end = handle.seek(SeekFrom::End(0)).ok()?;
        let _ = handle.seek(SeekFrom::Start(current));
        return Some(end as usize);
    }

    let file = file_state(file_ref)?;
    let path = get_string_field(file, "__path");
    if path.is_empty() {
        return None;
    }
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.len() as usize)
}

fn push_lines_iterator(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
    auto_close: bool,
) -> i32 {
    let mut env = Table::new();
    set_string_field(&mut env, gc, "__kind", "io.lines");
    set_bool_field(&mut env, gc, "__auto_close", auto_close);
    set_bool_field(&mut env, gc, "__dead", false);
    let env_ptr = &mut env as *mut Table;
    set_table_value(env_ptr, gc, "__file", &Value::Userdata(file_ref));
    let env_ref = gc.create(env);

    let mut iter = Function::new_c(lua_io_lines_iter);
    iter.set_env(Some(env_ref));
    let iter_ref = gc.create(iter);
    l.push_value(Value::Function(iter_ref));
    1
}

fn current_c_function_env(l: &LuaState) -> Option<GcRef<Table>> {
    let ci = l.current_call_info();
    match l.stack.at(ci.func) {
        Some(Value::Function(func_ref)) => {
            // SAFETY: the current call frame keeps its function live.
            unsafe { func_ref.as_ref() }.and_then(|func| func.env())
        }
        _ => None,
    }
}

fn push_error(l: &mut LuaState, gc: &mut GarbageCollector, message: &str) -> i32 {
    push_lua_string(l, gc, message);
    -1
}

fn file_arg(l: &LuaState) -> Option<GcRef<Userdata>> {
    match l.at(1) {
        Some(Value::Userdata(t)) => Some(*t),
        _ => None,
    }
}

fn string_arg(l: &LuaState, idx: i32) -> Option<String> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: argument strings are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.data().to_string())
        }
        _ => None,
    }
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        Some(Value::String(s)) => {
            // SAFETY: argument strings are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.and_then(|s| s.data().trim().parse::<f64>().ok())
        }
        _ => None,
    }
}

fn get_field(table: &Table, name: &str) -> Value {
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

fn get_string_field(table: &Table, name: &str) -> String {
    match get_field(table, name) {
        Value::String(s) => {
            // SAFETY: string value is owned by this live table.
            unsafe { s.as_ref() }
                .map(|s| s.data().to_string())
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn get_number_field(table: &Table, name: &str) -> f64 {
    match get_field(table, name) {
        Value::Number(n) => n,
        _ => 0.0,
    }
}

fn get_bool_field(table: &Table, name: &str) -> bool {
    match get_field(table, name) {
        Value::Boolean(value) => value,
        _ => false,
    }
}

fn set_string_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: &str) {
    let key = gc.create(GcString::new(name));
    let text = gc.create(GcString::new(value));
    table.set(&Value::String(key), &Value::String(text));
}

fn set_number_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: f64) {
    let key = gc.create(GcString::new(name));
    table.set(&Value::String(key), &Value::Number(value));
}

fn set_bool_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: bool) {
    let key = gc.create(GcString::new(name));
    table.set(&Value::String(key), &Value::Boolean(value));
}

fn push_lua_string(l: &mut LuaState, gc: &mut GarbageCollector, text: &str) {
    let s = gc.create(GcString::new(text));
    l.push_value(Value::String(s));
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

fn write_at(content: &str, pos: usize, appended: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let start = pos.min(chars.len());
    let replace_end = (start + appended.chars().count()).min(chars.len());

    let mut result = String::new();
    result.extend(chars[..start].iter());
    result.push_str(appended);
    result.extend(chars[replace_end..].iter());
    result
}

fn value_to_write_string(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => number_to_lua_string(*n),
        Value::String(s) => {
            // SAFETY: string arguments are kept alive on the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| s.data().to_string())
                .unwrap_or_default()
        }
        Value::Table(t) => format!("table: {:p}", t.as_ptr()),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => format!("userdata: {p:p}"),
    }
}

fn number_to_lua_string(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

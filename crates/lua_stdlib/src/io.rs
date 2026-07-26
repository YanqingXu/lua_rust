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

use std::io::{BufRead, Read, Seek, SeekFrom, Write};

const DIRECT_WRITE_THRESHOLD: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StandardStream {
    Stdin,
    Stdout,
    Stderr,
}

struct IoFileData {
    direct_handle: Option<std::fs::File>,
    standard_stream: Option<StandardStream>,
}

pub fn open_io(l: &mut LuaState, gc: &mut GarbageCollector) {
    let io_table = find_lib_table(l, "io");
    if io_table.is_null() {
        return;
    }

    let table_ptr = io_table.as_ptr() as *mut Table;
    let stdout = create_standard_file(gc, b"w", true, StandardStream::Stdout);
    let stdin = create_standard_file(gc, b"r", false, StandardStream::Stdin);
    let stderr = create_standard_file(gc, b"w", true, StandardStream::Stderr);
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
    let name_str = gc.create(GcString::from_bytes(name.as_bytes()));
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
                && key_str.as_bytes() == name.as_bytes()
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
    mode: &[u8],
    writable: bool,
) -> GcRef<Userdata> {
    create_file(gc, path, mode, writable, None)
}

fn create_standard_file(
    gc: &mut GarbageCollector,
    mode: &[u8],
    writable: bool,
    standard_stream: StandardStream,
) -> GcRef<Userdata> {
    create_file(gc, None, mode, writable, Some(standard_stream))
}

fn create_file(
    gc: &mut GarbageCollector,
    path: Option<String>,
    mode: &[u8],
    writable: bool,
    standard_stream: Option<StandardStream>,
) -> GcRef<Userdata> {
    let mut state = Table::new();
    let initial = if mode.contains(&b'a') || (mode.starts_with(b"r") && !mode.starts_with(b"w")) {
        path.as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let initial_len = initial.len();
    set_bytes_field(&mut state, gc, "__content", &initial);
    set_number_field(
        &mut state,
        gc,
        "__pos",
        if mode.contains(&b'a') {
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
        mode.starts_with(b"r") || mode.contains(&b'+'),
    );
    set_bytes_field(&mut state, gc, "__mode", mode);
    set_bytes_field(&mut state, gc, "__buffer", b"full");
    set_bool_field(&mut state, gc, "__buffer_explicit", false);
    set_bool_field(&mut state, gc, "__direct", false);
    set_bool_field(&mut state, gc, "__stdin_eof", false);
    if let Some(path) = path {
        set_bytes_field(&mut state, gc, "__path", path.as_bytes());
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
    let index_key = gc.create(GcString::from_bytes(b"__index"));
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
            standard_stream,
        });
    }
    userdata.set_metatable(Some(state_ref));
    gc.create(userdata)
}

fn set_table_value(table: *mut Table, gc: &mut GarbageCollector, key: &str, value: &Value) {
    let key = gc.create(GcString::from_bytes(key.as_bytes()));
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
    let key = gc.create(GcString::from_bytes(key.as_bytes()));
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

fn with_file_state<R>(
    file_ref: GcRef<Userdata>,
    access: impl for<'state> FnOnce(&'state Table) -> R,
) -> Option<R> {
    // SAFETY: file_ref is rooted by the active stack/table/function environment for
    // this call. The HRTB callback keeps the metatable borrow inside this function.
    let userdata = unsafe { file_ref.as_ref() }?;
    let metatable = userdata.metatable()?;
    // SAFETY: the userdata keeps its metatable alive for the callback's duration.
    let state = unsafe { metatable.as_ptr().as_ref() }?;
    Some(access(state))
}

fn with_file_state_mut<R>(
    file_ref: GcRef<Userdata>,
    access: impl for<'state> FnOnce(&'state mut Table) -> R,
) -> Option<R> {
    // SAFETY: file_ref is rooted for this call and IO operations are serialized on
    // the VM thread. The HRTB callback prevents the mutable reference from escaping.
    let userdata = unsafe { file_ref.as_ref() }?;
    let metatable = userdata.metatable()?;
    // SAFETY: the userdata keeps its metatable alive, and access is scoped to the
    // callback so no mutable table reference is returned to the caller.
    let state = unsafe { (metatable.as_ptr() as *mut Table).as_mut() }?;
    Some(access(state))
}

fn open_file_handle(
    gc: &mut GarbageCollector,
    path: &str,
    mode: &[u8],
) -> std::io::Result<GcRef<Userdata>> {
    let read_mode = mode.starts_with(b"r");
    let append_mode = mode.starts_with(b"a");
    let write_mode = mode.starts_with(b"w") || append_mode || mode.contains(&b'+');
    let binary_mode = mode.contains(&b'b');
    let normalized_mode = if binary_mode {
        mode.iter()
            .copied()
            .filter(|byte| *byte != b'b')
            .collect::<Vec<_>>()
    } else {
        mode.to_vec()
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
        if normalized_mode.starts_with(b"w") {
            options.truncate(true);
        }
        if append_mode {
            options.append(true);
        }
        if normalized_mode.contains(&b'+') {
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

    let file_ref = create_memory_file(gc, None, b"w+", true);
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
            let path = match gc_string_utf8(path_ref) {
                Ok(path) => path,
                Err(message) => return push_io_error_tuple(l, gc, message, 0),
            };
            match open_file_handle(gc, &path, b"r") {
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
    let Some(gc_ptr) = l.gc else {
        l.push_nil();
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let path = match utf8_string_arg(l, 1) {
        Ok(Some(path)) => path,
        Ok(None) => {
            l.push_nil();
            return 1;
        }
        Err(message) => return push_io_error_tuple(l, gc, message, 0),
    };
    let mode = bytes_arg(l, 2).unwrap_or_else(|| b"r".to_vec());
    if !mode.is_ascii() {
        return push_io_error_tuple(l, gc, "invalid file mode", 0);
    }
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
            let Some(closed) = with_file_state(file_ref, |state| get_bool_field(state, "__closed"))
            else {
                l.push_nil();
                return 1;
            };
            if closed {
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
            let path = match gc_string_utf8(path_ref) {
                Ok(path) => path,
                Err(message) => return push_io_error_tuple(l, gc, message, 0),
            };
            match open_file_handle(gc, &path, b"r") {
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
            let path = match gc_string_utf8(path_ref) {
                Ok(path) => path,
                Err(message) => return push_io_error_tuple(l, gc, message, 0),
            };
            match open_file_handle(gc, &path, b"w") {
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

    let mut appended = Vec::new();
    for idx in first_arg..=l.get_top() {
        let value = l.at(idx).cloned().unwrap_or(Value::Nil);
        appended.extend_from_slice(&value_to_write_bytes(&value));
    }

    let Some((closed, writable, pos, content, direct_write, buffer_mode)) =
        with_file_state(file_ref, |file| {
            let already_direct = get_bool_field(file, "__direct");
            let pos = get_number_field(file, "__pos").max(0.0) as usize;
            let content = if already_direct {
                Vec::new()
            } else {
                get_bytes_field(file, "__content")
            };
            let direct_write = should_write_direct(file, already_direct, &content, pos, &appended);
            (
                get_bool_field(file, "__closed"),
                get_bool_field(file, "__writable"),
                pos,
                content,
                direct_write,
                get_bytes_field(file, "__buffer"),
            )
        })
    else {
        l.push_nil();
        return 1;
    };

    if closed {
        if throw_on_error {
            return push_error(l, gc, "attempt to use a closed file");
        }
        l.push_nil();
        push_lua_string(l, gc, "file is closed");
        l.push_value(Value::Number(0.0));
        return 3;
    }

    if !writable {
        l.push_nil();
        push_lua_string(l, gc, "file is not open for writing");
        l.push_value(Value::Number(0.0));
        return 3;
    }

    if let Some(stream) = file_standard_stream(file_ref) {
        match write_standard_stream(stream, &appended) {
            Ok(()) => {
                let _ = with_file_state_mut(file_ref, |file| {
                    set_number_field(file, gc, "__pos", (pos + appended.len()) as f64);
                });
                l.push_value(Value::Userdata(file_ref));
                return 1;
            }
            Err(err) => {
                if throw_on_error {
                    return push_error(l, gc, &err.to_string());
                }
                l.push_nil();
                push_lua_string(l, gc, &err.to_string());
                l.push_value(Value::Number(err.raw_os_error().unwrap_or(0) as f64));
                return 3;
            }
        }
    }

    if direct_write {
        match write_direct(gc, file_ref, &content, pos, &appended) {
            Ok(new_pos) => {
                let _ = with_file_state_mut(file_ref, |file| {
                    set_number_field(file, gc, "__pos", new_pos as f64);
                });
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
    let new_pos = pos + appended.len();
    let _ = with_file_state_mut(file_ref, |file| {
        set_bytes_field(file, gc, "__content", &new_content);
        set_number_field(file, gc, "__pos", new_pos as f64);
    });

    if buffer_mode == b"no" || (buffer_mode == b"line" && appended.contains(&b'\n')) {
        let _ = flush_file_to_disk(file_ref);
    }

    l.push_value(Value::Userdata(file_ref));
    1
}

fn write_standard_stream(stream: StandardStream, bytes: &[u8]) -> std::io::Result<()> {
    match stream {
        StandardStream::Stdout => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(bytes)?;
            stdout.flush()
        }
        StandardStream::Stderr => {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(bytes)?;
            stderr.flush()
        }
        StandardStream::Stdin => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "standard input is not writable",
        )),
    }
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

    let whence = bytes_arg(l, 2).unwrap_or_else(|| b"cur".to_vec());
    let offset = number_arg(l, 3).unwrap_or(0.0) as isize;
    let Some((direct, buffered_len, current)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__direct"),
            get_bytes_field(file, "__content").len(),
            get_number_field(file, "__pos") as isize,
        )
    }) else {
        l.push_nil();
        return 1;
    };

    let content_len = if direct {
        direct_file_len(file_ref).unwrap_or(buffered_len) as isize
    } else {
        buffered_len as isize
    };
    let base = match whence.as_slice() {
        b"set" => 0,
        b"end" => content_len,
        _ => current,
    };
    let new_pos = (base + offset).clamp(0, content_len) as usize;
    let _ = with_file_state_mut(file_ref, |file| {
        set_number_field(file, gc, "__pos", new_pos as f64);
    });
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
    let Some(mode) = bytes_arg(l, 2) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Boolean(false));
        return 1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let updated = with_file_state_mut(file_ref, |file| match mode.as_slice() {
        b"no" | b"full" | b"line" => {
            set_bytes_field(file, gc, "__buffer", &mode);
            set_bool_field(file, gc, "__buffer_explicit", true);
            true
        }
        _ => false,
    })
    .unwrap_or(false);
    l.push_value(Value::Boolean(updated));
    1
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
    let text = match file_arg(l)
        .and_then(|file_ref| with_file_state(file_ref, |file| get_bool_field(file, "__closed")))
    {
        Some(true) => "file (closed)".to_string(),
        Some(false) => "file".to_string(),
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
            push_lua_bytes(l, gc, &line);
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
    let Some(closed) = with_file_state(file_ref, |file| get_bool_field(file, "__closed")) else {
        l.push_value(Value::Boolean(false));
        return 1;
    };

    if closed {
        return push_error(l, gc, "attempt to close a closed file");
    }

    if flush_file_to_disk(file_ref).is_err() {
        l.push_value(Value::Boolean(false));
        return 1;
    }

    close_direct_handle(file_ref);
    let _ = with_file_state_mut(file_ref, |file| {
        set_bool_field(file, gc, "__closed", true);
    });
    l.push_value(Value::Boolean(true));
    1
}

fn close_file_silent(gc: &mut GarbageCollector, file_ref: GcRef<Userdata>) -> Result<(), String> {
    let Some(closed) = with_file_state(file_ref, |file| get_bool_field(file, "__closed")) else {
        return Err("invalid file".to_string());
    };
    if closed {
        return Err("attempt to close a closed file".to_string());
    }
    flush_file_to_disk(file_ref).map_err(|err| err.to_string())?;
    close_direct_handle(file_ref);
    let Some(()) = with_file_state_mut(file_ref, |file| {
        set_bool_field(file, gc, "__closed", true);
    }) else {
        return Err("invalid file".to_string());
    };
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
    Bytes(Vec<u8>),
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

    let Some((readable, mut pos)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__readable"),
            get_number_field(file, "__pos").max(0.0) as usize,
        )
    }) else {
        l.push_nil();
        return 1;
    };
    if !readable {
        l.push_nil();
        return 1;
    }

    let mut values = Vec::new();
    for format in formats {
        if let Err(message) = prepare_standard_input(gc, file_ref, pos, &format) {
            return push_error(l, gc, &message);
        }
        let content = with_file_state(file_ref, |file| get_bytes_field(file, "__content"))
            .unwrap_or_default();
        let value = read_one(&content, pos, &format);
        pos = value.1;
        let result = value.0;
        let stop = matches!(result, ReadValue::Nil);
        values.push(result);
        if stop {
            break;
        }
    }

    let _ = with_file_state_mut(file_ref, |file| {
        set_number_field(file, gc, "__pos", pos as f64);
    });

    let count = values.len();
    for value in values {
        match value {
            ReadValue::Nil => l.push_nil(),
            ReadValue::Bytes(bytes) => push_lua_bytes(l, gc, &bytes),
            ReadValue::Number(number) => l.push_value(Value::Number(number)),
        }
    }
    count as i32
}

fn read_line_from_file(
    _l: &mut LuaState,
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
) -> Result<Option<Vec<u8>>, String> {
    ensure_file_readable(gc, file_ref)?;
    let Some((readable, pos)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__readable"),
            get_number_field(file, "__pos").max(0.0) as usize,
        )
    }) else {
        return Err("invalid file".to_string());
    };
    if !readable {
        return Ok(None);
    }
    prepare_standard_input(gc, file_ref, pos, &ReadFormat::Line)?;
    let Some(content) = with_file_state(file_ref, |file| get_bytes_field(file, "__content")) else {
        return Err("invalid file".to_string());
    };
    match read_one(&content, pos, &ReadFormat::Line) {
        (ReadValue::Bytes(line), new_pos) => {
            let _ = with_file_state_mut(file_ref, |file| {
                set_number_field(file, gc, "__pos", new_pos as f64);
            });
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
    let Some(closed) = with_file_state(file_ref, |file| get_bool_field(file, "__closed")) else {
        return Err("invalid file".to_string());
    };
    if closed {
        return Err("attempt to use a closed file".to_string());
    }
    refresh_file_from_disk(gc, file_ref);
    Ok(())
}

fn prepare_standard_input(
    gc: &mut GarbageCollector,
    file_ref: GcRef<Userdata>,
    pos: usize,
    format: &ReadFormat,
) -> Result<(), String> {
    if file_standard_stream(file_ref) != Some(StandardStream::Stdin) {
        return Ok(());
    }

    let Some((stdin_eof, content)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__stdin_eof"),
            get_bytes_field(file, "__content"),
        )
    }) else {
        return Err("invalid file".to_string());
    };
    if stdin_eof {
        return Ok(());
    }
    let available = content.len().saturating_sub(pos);
    let suffix = &content[pos.min(content.len())..];

    let mut bytes = Vec::new();
    let reached_eof = match format {
        ReadFormat::Line | ReadFormat::Number => {
            if suffix.contains(&b'\n') {
                return Ok(());
            }
            let read = std::io::stdin()
                .lock()
                .read_until(b'\n', &mut bytes)
                .map_err(|err| err.to_string())?;
            read == 0
        }
        ReadFormat::All => {
            std::io::stdin()
                .lock()
                .read_to_end(&mut bytes)
                .map_err(|err| err.to_string())?;
            true
        }
        ReadFormat::Bytes(count) => {
            let required = if *count == 0 { 1 } else { *count };
            if available >= required {
                return Ok(());
            }

            let mut remaining = required - available;
            let mut stdin = std::io::stdin().lock();
            let mut eof = false;
            while remaining > 0 {
                let mut chunk = vec![0; remaining.min(8192)];
                let read = stdin.read(&mut chunk).map_err(|err| err.to_string())?;
                if read == 0 {
                    eof = true;
                    break;
                }
                chunk.truncate(read);
                bytes.extend_from_slice(&chunk);
                remaining = remaining.saturating_sub(read);
            }
            eof
        }
    };

    let Some(()) = with_file_state_mut(file_ref, |file| {
        if !bytes.is_empty() {
            let mut updated = get_bytes_field(file, "__content");
            updated.extend_from_slice(&bytes);
            set_bytes_field(file, gc, "__content", &updated);
        }
        if reached_eof {
            set_bool_field(file, gc, "__stdin_eof", true);
        }
    }) else {
        return Err("invalid file".to_string());
    };
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
                let option = unsafe { s.as_ref() }
                    .map(|s| s.as_bytes())
                    .unwrap_or_default();
                match option {
                    b"*l" | b"*line" => formats.push(ReadFormat::Line),
                    b"*a" | b"*all" => formats.push(ReadFormat::All),
                    b"*n" | b"*number" => formats.push(ReadFormat::Number),
                    _ => return Err("invalid read option".to_string()),
                }
            }
            _ => return Err("invalid read option".to_string()),
        }
    }
    Ok(formats)
}

fn read_one(content: &[u8], pos: usize, format: &ReadFormat) -> (ReadValue, usize) {
    let pos = pos.min(content.len());
    match format {
        ReadFormat::Line => read_line_bytes(content, pos),
        ReadFormat::All => {
            let bytes = content[pos..].to_vec();
            (ReadValue::Bytes(bytes), content.len())
        }
        ReadFormat::Number => read_number_bytes(content, pos),
        ReadFormat::Bytes(count) => {
            if *count == 0 {
                if pos < content.len() {
                    (ReadValue::Bytes(Vec::new()), pos)
                } else {
                    (ReadValue::Nil, pos)
                }
            } else if pos >= content.len() {
                (ReadValue::Nil, pos)
            } else {
                let end = pos.saturating_add(*count).min(content.len());
                let bytes = content[pos..end].to_vec();
                (ReadValue::Bytes(bytes), end)
            }
        }
    }
}

fn read_line_bytes(content: &[u8], pos: usize) -> (ReadValue, usize) {
    if pos >= content.len() {
        return (ReadValue::Nil, pos);
    }
    let mut end = pos;
    while end < content.len() && content[end] != b'\n' {
        end += 1;
    }
    let mut line_end = end;
    if line_end > pos && content[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let bytes = content[pos..line_end].to_vec();
    let new_pos = if end < content.len() { end + 1 } else { end };
    (ReadValue::Bytes(bytes), new_pos)
}

fn read_number_bytes(content: &[u8], pos: usize) -> (ReadValue, usize) {
    let mut idx = pos;
    while idx < content.len() && content[idx].is_ascii_whitespace() {
        idx += 1;
    }
    let start = idx;
    if idx < content.len() && matches!(content[idx], b'+' | b'-') {
        idx += 1;
    }

    let mut digits_before_dot = 0;
    while idx < content.len() && content[idx].is_ascii_digit() {
        digits_before_dot += 1;
        idx += 1;
    }

    let mut digits_after_dot = 0;
    if idx < content.len() && content[idx] == b'.' {
        idx += 1;
        while idx < content.len() && content[idx].is_ascii_digit() {
            digits_after_dot += 1;
            idx += 1;
        }
    }

    if digits_before_dot == 0 && digits_after_dot == 0 {
        return (ReadValue::Nil, pos);
    }

    let mantissa_end = idx;
    if idx < content.len() && matches!(content[idx], b'e' | b'E') {
        let exp_start = idx;
        idx += 1;
        if idx < content.len() && matches!(content[idx], b'+' | b'-') {
            idx += 1;
        }
        let exp_digits_start = idx;
        while idx < content.len() && content[idx].is_ascii_digit() {
            idx += 1;
        }
        if exp_digits_start == idx {
            idx = exp_start;
        }
    }

    let token_end = idx.max(mantissa_end);
    let token = std::str::from_utf8(&content[start..token_end])
        .expect("numeric token contains only ASCII bytes");
    match token.parse::<f64>() {
        Ok(number) => (ReadValue::Number(number), token_end),
        Err(_) => (ReadValue::Nil, pos),
    }
}

fn refresh_file_from_disk(gc: &mut GarbageCollector, file_ref: GcRef<Userdata>) {
    let Some((writable, path, old_pos)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__writable"),
            get_utf8_field(file, "__path"),
            get_number_field(file, "__pos").max(0.0) as usize,
        )
    }) else {
        return;
    };
    if writable {
        return;
    }
    let Ok(Some(path)) = path else {
        return;
    };
    if let Ok(bytes) = std::fs::read(&path) {
        let len = bytes.len();
        let _ = with_file_state_mut(file_ref, |file| {
            set_bytes_field(file, gc, "__content", &bytes);
            set_number_field(file, gc, "__pos", old_pos.min(len) as f64);
        });
    }
}

fn flush_file_to_disk(file_ref: GcRef<Userdata>) -> std::io::Result<()> {
    let Some((closed, writable, direct, path, content)) = with_file_state(file_ref, |file| {
        (
            get_bool_field(file, "__closed"),
            get_bool_field(file, "__writable"),
            get_bool_field(file, "__direct"),
            get_utf8_field(file, "__path"),
            get_bytes_field(file, "__content"),
        )
    }) else {
        return Ok(());
    };
    if closed || !writable {
        return Ok(());
    }
    if let Some(stream) = file_standard_stream(file_ref) {
        return flush_standard_stream(stream);
    }
    if direct {
        return with_file_data_mut(file_ref, |data| {
            if let Some(handle) = data.direct_handle.as_mut() {
                handle.flush()?;
            }
            Ok::<(), std::io::Error>(())
        })
        .unwrap_or(Ok(()));
    }
    let Some(path) = path? else {
        return Ok(());
    };
    std::fs::write(path, content)
}

fn flush_standard_stream(stream: StandardStream) -> std::io::Result<()> {
    match stream {
        StandardStream::Stdout => std::io::stdout().lock().flush(),
        StandardStream::Stderr => std::io::stderr().lock().flush(),
        StandardStream::Stdin => Ok(()),
    }
}

fn should_write_direct(
    file: &Table,
    already_direct: bool,
    content: &[u8],
    pos: usize,
    appended: &[u8],
) -> bool {
    if get_bool_field(file, "__buffer_explicit") || get_bool_field(file, "__closed") {
        return false;
    }
    if !get_bool_field(file, "__writable") {
        return false;
    }
    let path = get_bytes_field(file, "__path");
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
    content: &[u8],
    pos: usize,
    appended: &[u8],
) -> std::io::Result<usize> {
    let Some((path, direct)) = with_file_state(file_ref, |file| {
        (
            get_utf8_field(file, "__path"),
            get_bool_field(file, "__direct"),
        )
    }) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file state is missing",
        ));
    };
    let Some(path) = path? else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file path is missing",
        ));
    };
    if !direct {
        std::fs::write(&path, content)?;
        let Some(()) = with_file_state_mut(file_ref, |file| {
            set_bool_field(file, gc, "__direct", true);
        }) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "file state is missing",
            ));
        };
    }

    if let Some(result) = with_file_data_mut(file_ref, |data| -> std::io::Result<()> {
        if data.direct_handle.is_none() {
            data.direct_handle = Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .read(true)
                    .truncate(false)
                    .open(&path)?,
            );
        }
        let handle = data.direct_handle.as_mut().expect("direct handle was set");
        handle.seek(SeekFrom::Start(pos as u64))?;
        handle.write_all(appended)?;
        Ok(())
    }) {
        result?;
    } else {
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(false)
            .open(&path)?;
        handle.seek(SeekFrom::Start(pos as u64))?;
        handle.write_all(appended)?;
    }
    Ok(pos + appended.len())
}

fn with_file_data<R>(
    file_ref: GcRef<Userdata>,
    access: impl for<'data> FnOnce(&'data IoFileData) -> R,
) -> Option<R> {
    // SAFETY: file_ref is rooted for this call. The HRTB callback prevents a
    // reference into the userdata payload from escaping.
    let userdata = unsafe { file_ref.as_ref() }?;
    // SAFETY: create_file constructs every IO userdata with IoFileData.
    let data = unsafe { userdata.data_as::<IoFileData>() }?;
    Some(access(data))
}

fn with_file_data_mut<R>(
    file_ref: GcRef<Userdata>,
    access: impl for<'data> FnOnce(&'data mut IoFileData) -> R,
) -> Option<R> {
    // SAFETY: file_ref is rooted for this call and IO operations are serialized on
    // the VM thread. The HRTB callback prevents mutable payload access from escaping.
    let userdata = unsafe { (file_ref.as_ptr() as *mut Userdata).as_mut() }?;
    // SAFETY: create_file constructs every IO userdata with IoFileData.
    let data = unsafe { userdata.data_as_mut::<IoFileData>() }?;
    Some(access(data))
}

fn file_standard_stream(file_ref: GcRef<Userdata>) -> Option<StandardStream> {
    with_file_data(file_ref, |data| data.standard_stream).flatten()
}

fn close_direct_handle(file_ref: GcRef<Userdata>) {
    let _ = with_file_data_mut(file_ref, |data| {
        data.direct_handle.take();
    });
}

fn direct_file_len(file_ref: GcRef<Userdata>) -> Option<usize> {
    if let Some(length) = with_file_data_mut(file_ref, |data| {
        let handle = data.direct_handle.as_mut()?;
        let current = handle.stream_position().ok()?;
        let end = handle.seek(SeekFrom::End(0)).ok()?;
        let _ = handle.seek(SeekFrom::Start(current));
        Some(end as usize)
    })
    .flatten()
    {
        return Some(length);
    }

    let path = with_file_state(file_ref, |file| get_utf8_field(file, "__path"))?;
    let Ok(Some(path)) = path else {
        return None;
    };
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
    set_bytes_field(&mut env, gc, "__kind", b"io.lines");
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

fn bytes_arg(l: &LuaState, idx: i32) -> Option<Vec<u8>> {
    match l.at(idx) {
        Some(Value::String(s)) => {
            // SAFETY: argument strings are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.map(|s| s.as_bytes().to_vec())
        }
        _ => None,
    }
}

fn utf8_string_arg(l: &LuaState, idx: i32) -> Result<Option<String>, &'static str> {
    match l.at(idx) {
        Some(Value::String(s)) => gc_string_utf8(*s).map(Some),
        _ => Ok(None),
    }
}

fn gc_string_utf8(value: GcRef<GcString>) -> Result<String, &'static str> {
    // SAFETY: callers only pass strings rooted by the active Lua stack/table.
    let Some(value) = (unsafe { value.as_ref() }) else {
        return Ok(String::new());
    };
    value
        .to_utf8()
        .map(str::to_owned)
        .map_err(|_| "file path must be valid UTF-8")
}

fn number_arg(l: &LuaState, idx: i32) -> Option<f64> {
    match l.at(idx) {
        Some(Value::Number(n)) => Some(*n),
        Some(Value::String(s)) => {
            // SAFETY: argument strings are kept alive on the active Lua stack.
            unsafe { s.as_ref() }.and_then(|s| {
                let bytes = trim_ascii_whitespace(s.as_bytes());
                std::str::from_utf8(bytes).ok()?.parse::<f64>().ok()
            })
        }
        _ => None,
    }
}

fn get_field(table: &Table, name: &str) -> Value {
    for (key, value) in table.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: keys are owned by this live table.
            && let Some(key_str) = unsafe { key_ref.as_ref() }
            && key_str.as_bytes() == name.as_bytes()
        {
            return value.clone();
        }
    }
    Value::Nil
}

fn get_bytes_field(table: &Table, name: &str) -> Vec<u8> {
    match get_field(table, name) {
        Value::String(s) => {
            // SAFETY: string value is owned by this live table.
            unsafe { s.as_ref() }
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn get_utf8_field(table: &Table, name: &str) -> std::io::Result<Option<String>> {
    let bytes = get_bytes_field(table, name);
    if bytes.is_empty() {
        return Ok(None);
    }
    String::from_utf8(bytes).map(Some).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file path must be valid UTF-8",
        )
    })
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

fn set_bytes_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: &[u8]) {
    let key = gc.create(GcString::from_bytes(name.as_bytes()));
    let text = gc.create(GcString::from_bytes(value));
    table.set(&Value::String(key), &Value::String(text));
}

fn set_number_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: f64) {
    let key = gc.create(GcString::from_bytes(name.as_bytes()));
    table.set(&Value::String(key), &Value::Number(value));
}

fn set_bool_field(table: &mut Table, gc: &mut GarbageCollector, name: &str, value: bool) {
    let key = gc.create(GcString::from_bytes(name.as_bytes()));
    table.set(&Value::String(key), &Value::Boolean(value));
}

fn push_lua_string(l: &mut LuaState, gc: &mut GarbageCollector, text: &str) {
    let s = gc.create(GcString::from_utf8_text(text));
    l.push_value(Value::String(s));
}

fn push_lua_bytes(l: &mut LuaState, gc: &mut GarbageCollector, bytes: &[u8]) {
    let s = gc.create(GcString::from_bytes(bytes));
    l.push_value(Value::String(s));
}

fn push_io_error_tuple(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    message: &str,
    error_code: i32,
) -> i32 {
    l.push_nil();
    push_lua_string(l, gc, message);
    l.push_value(Value::Number(f64::from(error_code)));
    3
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first()
        && first.is_ascii_whitespace()
    {
        bytes = rest;
    }
    while let Some((last, rest)) = bytes.split_last()
        && last.is_ascii_whitespace()
    {
        bytes = rest;
    }
    bytes
}

fn write_at(content: &[u8], pos: usize, appended: &[u8]) -> Vec<u8> {
    let start = pos.min(content.len());
    let replace_end = start.saturating_add(appended.len()).min(content.len());

    let mut result = Vec::with_capacity(
        start
            .saturating_add(appended.len())
            .saturating_add(content.len().saturating_sub(replace_end)),
    );
    result.extend_from_slice(&content[..start]);
    result.extend_from_slice(appended);
    result.extend_from_slice(&content[replace_end..]);
    result
}

fn value_to_write_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Nil => b"nil".to_vec(),
        Value::Boolean(b) => b.to_string().into_bytes(),
        Value::Number(n) => number_to_lua_string(*n).into_bytes(),
        Value::String(s) => {
            // SAFETY: string arguments are kept alive on the active Lua stack.
            unsafe { s.as_ref() }
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default()
        }
        Value::Table(t) => format!("table: {:p}", t.as_ptr()).into_bytes(),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()).into_bytes(),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()).into_bytes(),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()).into_bytes(),
        Value::LightUserdata(p) => format!("userdata: {p:p}").into_bytes(),
    }
}

fn number_to_lua_string(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userdata_payload_meets_io_file_data_alignment() {
        let mut userdata = Userdata::new(std::mem::size_of::<IoFileData>());
        assert_eq!(
            userdata.as_ptr() as usize % std::mem::align_of::<IoFileData>(),
            0
        );

        // SAFETY: the allocation has the asserted alignment and exact minimum
        // size, and no payload was previously constructed.
        unsafe {
            userdata.write_typed(IoFileData {
                direct_handle: None,
                standard_stream: None,
            });
        }
        // SAFETY: the preceding call constructed IoFileData in this payload.
        assert!(unsafe { userdata.data_as::<IoFileData>() }.is_some());
    }
}

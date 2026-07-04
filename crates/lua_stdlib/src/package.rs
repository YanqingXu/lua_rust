//! Package library.
//!
//! Current scope: Lua-file `require`, `module`, `package.loaded`, and `package.path`.

use std::path::{Path, PathBuf};

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::execute::call_value;
use lua_vm::state::LuaState;

const DEFAULT_PATH: &str = "?.lua;?/init.lua";
const DEFAULT_CPATH: &str = "";

pub fn open_package(l: &mut LuaState, gc: &mut GarbageCollector) {
    let package = ensure_package_table(l, gc);
    if package.is_null() {
        return;
    }

    let loaded = ensure_loaded_table(l, gc, package);
    let preload = ensure_preload_table(l, gc, package);
    preload_global_libraries(l, gc, loaded);

    let default_path = lua_string_value(l, gc, DEFAULT_PATH);
    set_table_string(l, gc, package, "path", &default_path);
    let default_cpath = lua_string_value(l, gc, DEFAULT_CPATH);
    set_table_string(l, gc, package, "cpath", &default_cpath);
    register_package_function(gc, package, "loadlib", lua_package_loadlib);
    register_package_function(gc, package, "seeall", lua_package_seeall);

    if let Some(global) = l.global_table {
        let global_ptr = global.as_ptr() as *mut Table;
        register_global(
            gc,
            global_ptr,
            "require",
            lua_package_require,
            Some(package),
        );
        register_global(gc, global_ptr, "module", lua_package_module, Some(package));
    }

    // Keep the preload table live and visible even when no preloaders are registered yet.
    let _ = preload;
}

pub fn add_script_directory_to_path(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    source_name: &str,
) {
    let source_name = source_name.strip_prefix('@').unwrap_or(source_name);
    let Some(dir) = Path::new(source_name)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
    else {
        return;
    };

    let package = ensure_package_table(l, gc);
    if package.is_null() {
        return;
    }

    let dir = dir.to_string_lossy();
    let prefix = format!(
        "{dir}{}?.lua;{dir}{}?{}init.lua",
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    let current = table_string_field(package, "path").unwrap_or_else(|| DEFAULT_PATH.to_string());
    let path = if current.is_empty() {
        prefix
    } else {
        format!("{prefix};{current}")
    };
    let path_value = lua_string_value(l, gc, &path);
    set_table_string(l, gc, package, "path", &path_value);
}

unsafe extern "C" fn lua_package_require(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(Value::String(module_ref)) = l.at(1).cloned() else {
        return raise_string(l, "bad argument #1 to 'require' (string expected)");
    };
    // SAFETY: module_ref is the first argument on the active Lua stack.
    let module_name = match unsafe { module_ref.as_ref() } {
        Some(name) => name.data().to_string(),
        None => return raise_string(l, "invalid module name"),
    };

    let Some(gc_ptr) = l.gc else {
        return raise_string(l, "require unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let package = active_package_table(l, gc);
    let loaded = ensure_loaded_table(l, gc, package);
    let module_key = Value::String(module_ref);

    let cached = table_get(loaded, &module_key);
    if !cached.is_nil() && !matches!(cached, Value::Boolean(false)) {
        l.push_value(cached);
        return 1;
    }

    let loader = match preload_loader(l, gc, package, &module_name) {
        Some(loader) => loader,
        None => {
            let (path, source) = match find_module_source(package, &module_name) {
                Ok(found) => found,
                Err(message) => return raise_string(l, &message),
            };

            let func_ref =
                match compile_chunk_function(l, gc, &source, &format!("@{}", path.display())) {
                    Ok(func_ref) => func_ref,
                    Err(message) => return raise_string(l, &message),
                };
            Value::Function(func_ref)
        }
    };

    match call_value(l, gc, loader, &[Value::String(module_ref)], None) {
        Ok(results) => {
            if let Some(result) = results.first()
                && !result.is_nil()
            {
                table_set(l, gc, loaded, &module_key, result);
            }

            let loaded_value = table_get(loaded, &module_key);
            if loaded_value.is_nil() {
                table_set(l, gc, loaded, &module_key, &Value::Boolean(true));
            }

            l.push_value(table_get(loaded, &module_key));
            1
        }
        Err(err) => {
            push_runtime_error_value(l, gc, &err);
            -1
        }
    }
}

unsafe extern "C" fn lua_package_loadlib(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let path = value_to_string(l.at(1).cloned().unwrap_or(Value::Nil));
    l.push_value(Value::Nil);
    let _ = push_lua_string(l, &format!("dynamic libraries not supported: {path}"));
    let _ = push_lua_string(l, "absent");
    3
}

unsafe extern "C" fn lua_package_seeall(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(Value::Table(module_ref)) = l.at(1).cloned() else {
        return raise_string(l, "bad argument #1 to 'seeall' (table expected)");
    };
    let Some(gc_ptr) = l.gc else {
        return raise_string(l, "seeall unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let global = l.global_table;

    if let Some(global) = global {
        set_table_string(l, gc, module_ref, "_G", &Value::Table(global));
        let mt = {
            // SAFETY: module_ref is an active argument.
            unsafe { module_ref.as_ref() }.and_then(|module| module.metatable())
        }
        .unwrap_or_else(|| gc.create(Table::new()));
        set_table_string(l, gc, mt, "__index", &Value::Table(global));
        // SAFETY: module_ref is an active argument and VM is single-threaded.
        unsafe { &mut *(module_ref.as_ptr() as *mut Table) }.set_metatable(Some(mt));
    }
    0
}

unsafe extern "C" fn lua_package_module(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(Value::String(name_ref)) = l.at(1).cloned() else {
        return raise_string(l, "bad argument #1 to 'module' (string expected)");
    };
    // SAFETY: name_ref is the first argument on the active Lua stack.
    let Some(module_name) = (unsafe { name_ref.as_ref() }).map(|s| s.data().to_string()) else {
        return raise_string(l, "invalid module name");
    };
    let Some(gc_ptr) = l.gc else {
        return raise_string(l, "module unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };

    let package = active_package_table(l, gc);
    let loaded = ensure_loaded_table(l, gc, package);
    let module_key = Value::String(name_ref);
    let module_ref = match module_table(l, gc, loaded, &module_name, &module_key) {
        Ok(module_ref) => module_ref,
        Err(message) => return raise_string(l, &message),
    };

    set_module_metadata(l, gc, module_ref, &module_name);
    table_set(l, gc, loaded, &module_key, &Value::Table(module_ref));

    let options: Vec<Value> = (2..=l.get_top())
        .filter_map(|idx| l.at(idx).cloned())
        .collect();
    for option in options {
        if !matches!(option, Value::Function(_)) {
            return raise_string(l, "module option must be a function");
        }
        if let Err(err) = call_value(l, gc, option, &[Value::Table(module_ref)], Some(0)) {
            push_runtime_error_value(l, gc, &err);
            return -1;
        }
    }

    if !set_caller_env(l, module_ref) {
        return raise_string(l, "module has no caller environment");
    }

    0
}

fn find_module_source(
    package: GcRef<Table>,
    module_name: &str,
) -> Result<(PathBuf, String), String> {
    let path = table_string_field(package, "path").unwrap_or_else(|| DEFAULT_PATH.to_string());
    let module_path = module_path_name(module_name);
    let mut attempted = Vec::new();

    for pattern in path.split(';').filter(|pattern| !pattern.is_empty()) {
        let candidate = pattern.replace('?', &module_path);
        let candidate_path = PathBuf::from(&candidate);
        attempted.push(candidate);
        if candidate_path.is_file() {
            let source = read_lua_source_file(&candidate_path).map_err(|err| err.to_string())?;
            return Ok((candidate_path, source));
        }
    }

    Err(format!(
        "module '{module_name}' not found: {}",
        attempted.join("; ")
    ))
}

fn module_path_name(module_name: &str) -> String {
    module_name
        .chars()
        .map(|ch| {
            if ch == '.' {
                std::path::MAIN_SEPARATOR
            } else {
                ch
            }
        })
        .collect()
}

fn read_lua_source_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(bytes.iter().map(|byte| char::from(*byte)).collect())
}

fn compile_chunk_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    source: &str,
    chunk_name: &str,
) -> Result<GcRef<Function>, String> {
    let mut parser = Parser::new(source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("{chunk_name}:{}:{}: {}", err.line, err.column, err.message))?;

    let generator = CodeGenerator::new(gc);
    let proto = generator
        .generate(&chunk, chunk_name)
        .map_err(|err| format!("{chunk_name}:{err}"))?;

    let proto_ref = gc.create(proto);
    let func_ref = gc.create(Function::new_lua(proto_ref));
    if l.gc.is_none() {
        l.gc = Some(gc as *mut GarbageCollector);
    }
    Ok(func_ref)
}

fn ensure_package_table(l: &mut LuaState, gc: &mut GarbageCollector) -> GcRef<Table> {
    if let Some(Value::Table(package)) = global_value(l, "package") {
        return package;
    }

    let package = gc.create(Table::new());
    set_global_value(l, gc, "package", &Value::Table(package));
    package
}

fn ensure_loaded_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
) -> GcRef<Table> {
    if let Value::Table(loaded) = table_get_string(l, gc, package, "loaded") {
        return loaded;
    }

    let loaded = gc.create(Table::new());
    set_table_string(l, gc, package, "loaded", &Value::Table(loaded));
    loaded
}

fn ensure_preload_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
) -> GcRef<Table> {
    if let Value::Table(preload) = table_get_string(l, gc, package, "preload") {
        return preload;
    }

    let preload = gc.create(Table::new());
    set_table_string(l, gc, package, "preload", &Value::Table(preload));
    preload
}

fn active_package_table(l: &mut LuaState, gc: &mut GarbageCollector) -> GcRef<Table> {
    current_c_function_env(l).unwrap_or_else(|| ensure_package_table(l, gc))
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

fn preload_global_libraries(l: &mut LuaState, gc: &mut GarbageCollector, loaded: GcRef<Table>) {
    for name in [
        "_G",
        "math",
        "io",
        "os",
        "string",
        "table",
        "debug",
        "coroutine",
        "package",
    ] {
        if let Some(value) = global_value(l, name)
            && !value.is_nil()
        {
            let key = lua_string_value(l, gc, name);
            table_set(l, gc, loaded, &key, &value);
        }
    }
}

fn register_global(
    gc: &mut GarbageCollector,
    table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
    env: Option<GcRef<Table>>,
) {
    let name_str = gc.create(GcString::new(name));
    let mut function = Function::new_c(func);
    function.set_env(env);
    let func_obj = gc.create(function);
    // SAFETY: table points to the GC-rooted global table during library registration.
    unsafe {
        (*table).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

fn register_package_function(
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: package is rooted through the global table during library registration.
    unsafe {
        let package = &mut *(package.as_ptr() as *mut Table);
        package.set(&Value::String(name_str), &Value::Function(func_obj));
    }
}

fn preload_loader(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
    module_name: &str,
) -> Option<Value> {
    let preload = ensure_preload_table(l, gc, package);
    let key = lua_string_value(l, gc, module_name);
    match table_get(preload, &key) {
        loader @ Value::Function(_) => Some(loader),
        _ => None,
    }
}

fn module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    loaded: GcRef<Table>,
    module_name: &str,
    module_key: &Value,
) -> Result<GcRef<Table>, String> {
    if let Value::Table(module_ref) = table_get(loaded, module_key) {
        ensure_global_module_path(l, gc, module_name, module_ref)?;
        return Ok(module_ref);
    }

    let module_ref = ensure_global_module_table(l, gc, module_name)?;
    Ok(module_ref)
}

fn ensure_global_module_path(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &str,
    module_ref: GcRef<Table>,
) -> Result<(), String> {
    let Some((parent, leaf)) = module_parent_table(l, gc, module_name)? else {
        set_global_value(l, gc, module_name, &Value::Table(module_ref));
        return Ok(());
    };
    let key = lua_string_value(l, gc, &leaf);
    table_set(l, gc, parent, &key, &Value::Table(module_ref));
    Ok(())
}

fn ensure_global_module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &str,
) -> Result<GcRef<Table>, String> {
    if let Some(value) = global_value(l, module_name) {
        return match value {
            Value::Table(module_ref) => Ok(module_ref),
            Value::Nil => Ok(create_named_module_table(l, gc, module_name)?),
            _ => Err(format!("name conflict for module '{module_name}'")),
        };
    }

    create_named_module_table(l, gc, module_name)
}

fn create_named_module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &str,
) -> Result<GcRef<Table>, String> {
    let Some((parent, leaf)) = module_parent_table(l, gc, module_name)? else {
        let module_ref = gc.create(Table::new());
        set_global_value(l, gc, module_name, &Value::Table(module_ref));
        return Ok(module_ref);
    };

    let leaf_key = lua_string_value(l, gc, &leaf);
    let existing = table_get(parent, &leaf_key);
    match existing {
        Value::Table(module_ref) => Ok(module_ref),
        Value::Nil => {
            let module_ref = gc.create(Table::new());
            table_set(l, gc, parent, &leaf_key, &Value::Table(module_ref));
            Ok(module_ref)
        }
        _ => Err(format!("name conflict for module '{module_name}'")),
    }
}

fn module_parent_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &str,
) -> Result<Option<(GcRef<Table>, String)>, String> {
    let Some((first, rest)) = module_name.split_once('.') else {
        return Ok(None);
    };

    let root = match global_value(l, first) {
        Some(Value::Table(root)) => root,
        Some(Value::Nil) | None => {
            let root = gc.create(Table::new());
            set_global_value(l, gc, first, &Value::Table(root));
            root
        }
        Some(_) => return Err(format!("name conflict for module '{module_name}'")),
    };

    let mut current = root;
    let mut parts = rest.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok(Some((current, part.to_string())));
        }

        let key = lua_string_value(l, gc, part);
        match table_get(current, &key) {
            Value::Table(next) => current = next,
            Value::Nil => {
                let next = gc.create(Table::new());
                table_set(l, gc, current, &key, &Value::Table(next));
                current = next;
            }
            _ => return Err(format!("name conflict for module '{module_name}'")),
        }
    }

    Ok(None)
}

fn set_module_metadata(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_ref: GcRef<Table>,
    module_name: &str,
) {
    let name = lua_string_value(l, gc, module_name);
    set_table_string(l, gc, module_ref, "_NAME", &name);
    set_table_string(l, gc, module_ref, "_M", &Value::Table(module_ref));
    let package_name = module_package_name(module_name);
    let package = lua_string_value(l, gc, &package_name);
    set_table_string(l, gc, module_ref, "_PACKAGE", &package);
}

fn module_package_name(module_name: &str) -> String {
    module_name
        .rfind('.')
        .map(|idx| module_name[..=idx].to_string())
        .unwrap_or_default()
}

fn set_caller_env(l: &mut LuaState, module_ref: GcRef<Table>) -> bool {
    if l.current_ci == 0 {
        l.chunk_env = Some(module_ref);
        return true;
    }
    let caller_idx = l.current_ci - 1;
    let Some(ci) = l.call_stack.get(caller_idx) else {
        return false;
    };
    if ci.func == ci.base {
        l.chunk_env = Some(module_ref);
        return true;
    }
    let func_ref = match l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) {
        Value::Function(func_ref) => func_ref,
        _ => {
            l.chunk_env = Some(module_ref);
            return true;
        }
    };
    // SAFETY: the caller frame keeps the function live.
    unsafe { &mut *(func_ref.as_ptr() as *mut Function) }.set_env(Some(module_ref));
    true
}

fn table_get_string(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &str,
) -> Value {
    let key = lua_string_value(l, gc, key);
    table_get(table, &key)
}

fn table_string_field(table: GcRef<Table>, key: &str) -> Option<String> {
    // SAFETY: package table is reachable from the global table during package operations.
    let table_obj = unsafe { table.as_ref() }?;
    for (field_key, value) in table_obj.hash_entries() {
        if let Value::String(key_ref) = field_key
            // SAFETY: keys are held by the table being inspected.
            && let Some(key_str) = unsafe { key_ref.as_ref() }
            && key_str.data() == key
            && let Value::String(value_ref) = value
        {
            // SAFETY: values are held by the table being inspected.
            return unsafe { value_ref.as_ref() }.map(|value| value.data().to_string());
        }
    }
    None
}

fn set_table_string(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &str,
    value: &Value,
) {
    let key = lua_string_value(l, gc, key);
    table_set(l, gc, table, &key, value);
}

fn table_get(table: GcRef<Table>, key: &Value) -> Value {
    if table.is_null() {
        return Value::Nil;
    }
    // SAFETY: table is reachable from globals/package.loaded during require.
    unsafe { table.as_ref() }
        .map(|table| table.get(key))
        .unwrap_or(Value::Nil)
}

fn table_set(
    _l: &mut LuaState,
    _gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &Value,
    value: &Value,
) {
    if table.is_null() {
        return;
    }
    let table_ptr = table.as_ptr() as *mut Table;
    // SAFETY: table is reachable from globals/package.loaded during require.
    unsafe {
        (*table_ptr).set(key, value);
    }
}

fn global_value(l: &LuaState, name: &str) -> Option<Value> {
    let global = l.global_table?;
    // SAFETY: global table is rooted by LuaState.
    let global_obj = unsafe { global.as_ref() }?;
    for (key, value) in global_obj.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: key is held by the rooted global table.
            && let Some(key_str) = unsafe { key_ref.as_ref() }
            && key_str.data() == name
        {
            return Some(value.clone());
        }
    }
    None
}

fn set_global_value(l: &mut LuaState, gc: &mut GarbageCollector, name: &str, value: &Value) {
    let Some(global) = l.global_table else {
        return;
    };
    let key = lua_string_value(l, gc, name);
    let global_ptr = global.as_ptr() as *mut Table;
    // SAFETY: global table is rooted by LuaState.
    unsafe {
        (*global_ptr).set(&key, value);
    }
}

fn lua_string_value(l: &mut LuaState, gc: &mut GarbageCollector, text: &str) -> Value {
    Value::String(intern_string(l, gc, text))
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    let Some(gc_ptr) = l.gc else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let value = lua_string_value(l, gc, text);
    l.push_value(value);
    true
}

fn intern_string(l: &mut LuaState, gc: &mut GarbageCollector, text: &str) -> GcRef<GcString> {
    if let Some(pool_ptr) = l.string_pool {
        // SAFETY: string_pool is installed from a live StringPool owned by the host.
        let pool = unsafe { &mut *pool_ptr };
        pool.find(text).unwrap_or_else(|| pool.intern(gc, text))
    } else {
        gc.create(GcString::new(text))
    }
}

fn value_to_string(value: Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                format!("{n:.0}")
            } else {
                n.to_string()
            }
        }
        Value::String(s) => {
            // SAFETY: string value is an active argument while package.loadlib runs.
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

fn push_runtime_error_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    err: &lua_vm::RuntimeError,
) {
    if let Some(value) = err.error_value() {
        l.push_value(value);
    } else {
        let message = lua_string_value(l, gc, &err.message);
        l.push_value(message);
    }
}

fn raise_string(l: &mut LuaState, message: &str) -> i32 {
    let Some(gc_ptr) = l.gc else {
        l.push_value(Value::Nil);
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let message = lua_string_value(l, gc, message);
    l.push_value(message);
    -1
}

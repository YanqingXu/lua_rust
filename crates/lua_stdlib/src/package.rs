//! Package library.
//!
//! Current scope: Lua-file `require`, `module`, `package.loaded`, and `package.path`.

use std::path::{Path, PathBuf};

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::execute::call_value_with_results;
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

    crate::registration::set_string(l, gc, package, b"path", DEFAULT_PATH.as_bytes())
        .expect("package.path publication must remain collector-valid");
    crate::registration::set_string(l, gc, package, b"cpath", DEFAULT_CPATH.as_bytes())
        .expect("package.cpath publication must remain collector-valid");
    register_package_function(l, gc, package, "loadlib", lua_package_loadlib);
    register_package_function(l, gc, package, "seeall", lua_package_seeall);

    if let Some(global) = l.global_table {
        register_global(l, gc, global, "require", lua_package_require, Some(package));
        register_global(l, gc, global, "module", lua_package_module, Some(package));
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
    let current =
        table_string_field(l, package, "path").unwrap_or_else(|| DEFAULT_PATH.to_string());
    let path = if current.is_empty() {
        prefix
    } else {
        format!("{prefix};{current}")
    };
    crate::registration::set_string(l, gc, package, b"path", path.as_bytes())
        .expect("package.path update must remain collector-valid");
}

unsafe extern "C" fn lua_package_require(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(Value::String(module_ref)) = l.at(1).cloned() else {
        return raise_string(l, "bad argument #1 to 'require' (string expected)");
    };
    if l.copy_string_bytes(module_ref).is_err() {
        return raise_string(l, "invalid module name");
    }

    let Some(gc_ptr) = l.active_gc_ptr() else {
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

    let mut compiled_loader_on_stack = false;
    let loader = match preload_loader(l, gc, package, &module_key) {
        Some(loader) => loader,
        None => {
            // `package.loaded` and `package.preload` are Lua tables, so their
            // keys remain arbitrary byte strings. UTF-8 is required only when
            // the unresolved name crosses into the host filesystem.
            let module_name = match l
                .with_string_bytes(module_ref, |bytes| {
                    std::str::from_utf8(bytes).ok().map(str::to_owned)
                })
                .ok()
                .flatten()
            {
                Some(name) => name,
                None => return raise_string(l, "module name must be valid UTF-8"),
            };
            let (path, source) = match find_module_source(l, package, &module_name) {
                Ok(found) => found,
                Err(message) => return raise_string(l, &message),
            };

            let func_ref =
                match compile_chunk_function(l, gc, &source, &format!("@{}", path.display())) {
                    Ok(func_ref) => func_ref,
                    Err(message) => return raise_string(l, &message),
                };
            compiled_loader_on_stack = true;
            Value::Function(func_ref)
        }
    };

    let execution = call_value_with_results(
        l,
        gc,
        loader,
        &[Value::String(module_ref)],
        None,
        |l, gc, results| {
            if compiled_loader_on_stack {
                let _ = l.pop();
            }
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
        },
    );
    if compiled_loader_on_stack && execution.is_err() {
        let _ = l.pop();
    }
    match execution {
        Ok(()) => 1,
        Err(err) => {
            push_runtime_error_value(l, gc, &err);
            -1
        }
    }
}

unsafe extern "C" fn lua_package_loadlib(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let path = value_to_string(l, l.at(1).cloned().unwrap_or(Value::Nil));
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
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return raise_string(l, "seeall unavailable without an active GC");
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let global = l.global_table;

    if let Some(global) = global {
        crate::registration::set_value(l, gc, module_ref, b"_G", &Value::Table(global))
            .expect("module _G publication must remain collector-valid");
        let mt = {
            // SAFETY: module_ref is an active argument.
            unsafe { module_ref.as_ref() }.and_then(|module| module.metatable())
        };
        if let Some(mt) = mt {
            crate::registration::set_value(l, gc, mt, b"__index", &Value::Table(global))
                .expect("module __index publication must remain collector-valid");
            crate::registration::set_metatable(gc, module_ref, mt)
                .expect("module metatable publication must remain collector-valid");
        } else {
            gc.with_publication(|transaction| {
                let module = transaction
                    .protect(module_ref)
                    .expect("module argument must remain collector-valid");
                let _global = transaction
                    .protect(global)
                    .expect("global Table must remain collector-valid");
                let metatable = transaction.alloc(Table::new());
                let index = crate::registration::rooted_bytes(l, transaction, b"__index")
                    .expect("module __index key must remain collector-valid");
                transaction
                    .set_table_value(&metatable, &index, &Value::Table(global))
                    .expect("module __index edge must remain collector-valid");
                transaction
                    .set_table_metatable(&module, Some(&metatable))
                    .expect("module metatable edge must remain collector-valid");
            });
        }
    }
    0
}

unsafe extern "C" fn lua_package_module(l_ptr: *mut std::ffi::c_void) -> i32 {
    // SAFETY: l_ptr is the LuaState pointer passed by the VM CALL handler.
    let l = unsafe { &mut *(l_ptr as *mut LuaState) };
    let Some(Value::String(name_ref)) = l.at(1).cloned() else {
        return raise_string(l, "bad argument #1 to 'module' (string expected)");
    };
    let module_name = match l.copy_string_bytes(name_ref) {
        Ok(name) => name,
        Err(_) => return raise_string(l, "invalid module name"),
    };
    let Some(gc_ptr) = l.active_gc_ptr() else {
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

    set_module_metadata(l, gc, module_ref, &module_name, &module_key);
    table_set(l, gc, loaded, &module_key, &Value::Table(module_ref));

    if !set_caller_env(l, module_ref) {
        return raise_string(l, "module has no caller environment");
    }

    let options: Vec<Value> = (2..=l.get_top())
        .filter_map(|idx| l.at(idx).cloned())
        .collect();
    for option in options {
        if !matches!(option, Value::Function(_)) {
            return raise_string(l, "module option must be a function");
        }
        if let Err(err) = call_value_with_results(
            l,
            gc,
            option,
            &[Value::Table(module_ref)],
            Some(0),
            |_, _, _| (),
        ) {
            push_runtime_error_value(l, gc, &err);
            return -1;
        }
    }

    0
}

fn find_module_source(
    l: &LuaState,
    package: GcRef<Table>,
    module_name: &str,
) -> Result<(PathBuf, Vec<u8>), String> {
    let path = table_string_field(l, package, "path").unwrap_or_else(|| DEFAULT_PATH.to_string());
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

fn read_lua_source_file(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

fn compile_chunk_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    source: &[u8],
    chunk_name: &str,
) -> Result<GcRef<Function>, String> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("{chunk_name}:{}:{}: {}", err.line, err.column, err.message))?;
    let pool_ptr = l
        .active_string_pool_ptr()
        .ok_or_else(|| format!("{chunk_name}: loader unavailable without an active StringPool"))?;

    let func_ref = gc.with_publication(|transaction| {
        // SAFETY: the pool belongs to the dynamically scoped Runtime turn.
        let generator =
            CodeGenerator::new_in_publication_with_pool(transaction, unsafe { &mut *pool_ptr });
        let proto = generator
            .generate(&chunk, chunk_name)
            .map_err(|err| format!("{chunk_name}:{err}"))?;
        let proto = transaction.alloc(proto);
        let function = transaction
            .alloc_lua_function(&proto)
            .map_err(|err| format!("{chunk_name}: invalid loader Function: {err}"))?;
        // SAFETY: the callback publishes the loader on the active require
        // stack before releasing its temporary object root.
        unsafe {
            transaction.publish_function_value(function, |value| {
                let Value::Function(function) = value.clone() else {
                    unreachable!("typed Function publication produced another Value kind");
                };
                l.push_value(value);
                function
            })
        }
        .map_err(|err| format!("{chunk_name}: invalid loader stack publication: {err}"))
    })?;
    Ok(func_ref)
}

fn ensure_package_table(l: &mut LuaState, gc: &mut GarbageCollector) -> GcRef<Table> {
    if let Some(Value::Table(package)) = global_value(l, "package") {
        return package;
    }

    let Some(global) = l.global_table else {
        return GcRef::null();
    };
    crate::registration::publish_new_table(l, gc, global, b"package")
        .expect("package Table publication must remain collector-valid")
}

fn ensure_loaded_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
) -> GcRef<Table> {
    crate::registration::ensure_table(l, gc, package, b"loaded")
        .expect("package.loaded publication must remain collector-valid")
}

fn ensure_preload_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
) -> GcRef<Table> {
    crate::registration::ensure_table(l, gc, package, b"preload")
        .expect("package.preload publication must remain collector-valid")
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
            crate::registration::set_value(l, gc, loaded, name.as_bytes(), &value)
                .expect("package.loaded preload publication must remain collector-valid");
        }
    }
}

fn register_global(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
    env: Option<GcRef<Table>>,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), func, env)
        .expect("package global Function publication must remain collector-valid");
}

fn register_package_function(
    state: &LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    crate::registration::register_c_function(state, gc, package, name.as_bytes(), func, None)
        .expect("package Function publication must remain collector-valid");
}

fn preload_loader(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    package: GcRef<Table>,
    module_key: &Value,
) -> Option<Value> {
    let preload = ensure_preload_table(l, gc, package);
    match table_get(preload, module_key) {
        loader @ Value::Function(_) => Some(loader),
        _ => None,
    }
}

fn module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    loaded: GcRef<Table>,
    module_name: &[u8],
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
    module_name: &[u8],
    module_ref: GcRef<Table>,
) -> Result<(), String> {
    let fields = module_name_fields(module_name)?;
    let Some(global) = l.global_table else {
        return Err("module has no global table".to_string());
    };
    let mut parent = global;
    for (index, field) in fields.iter().enumerate() {
        if index + 1 == fields.len() {
            crate::registration::set_value(l, gc, parent, field, &Value::Table(module_ref))
                .map_err(|error| format!("invalid module path publication: {error}"))?;
            return Ok(());
        }
        match crate::registration::table_value_by_bytes(gc, parent, field)? {
            Value::Table(next) => parent = next,
            Value::Nil => {
                parent = crate::registration::publish_new_table(l, gc, parent, field)?;
            }
            _ => return Err(module_name_conflict(module_name)),
        }
    }
    Ok(())
}

fn ensure_global_module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &[u8],
) -> Result<GcRef<Table>, String> {
    create_named_module_table(l, gc, module_name)
}

fn create_named_module_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_name: &[u8],
) -> Result<GcRef<Table>, String> {
    let fields = module_name_fields(module_name)?;
    let Some(global) = l.global_table else {
        return Err("module has no global table".to_string());
    };
    let mut parent = global;
    for (index, field) in fields.iter().enumerate() {
        let existing = crate::registration::table_value_by_bytes(gc, parent, field)?;
        if index + 1 == fields.len() {
            return match existing {
                Value::Table(module_ref) => Ok(module_ref),
                Value::Nil => crate::registration::publish_new_table(l, gc, parent, field),
                _ => Err(module_name_conflict(module_name)),
            };
        }
        match existing {
            Value::Table(next) => parent = next,
            Value::Nil => {
                parent = crate::registration::publish_new_table(l, gc, parent, field)?;
            }
            _ => return Err(module_name_conflict(module_name)),
        }
    }

    Err(module_name_conflict(module_name))
}

fn set_module_metadata(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    module_ref: GcRef<Table>,
    module_name: &[u8],
    module_key: &Value,
) {
    crate::registration::set_value(l, gc, module_ref, b"_NAME", module_key)
        .expect("module _NAME publication must remain collector-valid");
    crate::registration::set_value(l, gc, module_ref, b"_M", &Value::Table(module_ref))
        .expect("module _M publication must remain collector-valid");
    let package_name = module_package_name(module_name);
    crate::registration::set_string(l, gc, module_ref, b"_PACKAGE", package_name)
        .expect("module _PACKAGE publication must remain collector-valid");
}

fn module_package_name(module_name: &[u8]) -> &[u8] {
    module_name
        .iter()
        .rposition(|&byte| byte == b'.')
        .map_or(&[], |index| &module_name[..=index])
}

fn module_name_fields(module_name: &[u8]) -> Result<Vec<&[u8]>, String> {
    let fields: Vec<_> = module_name.split(|&byte| byte == b'.').collect();
    if fields.iter().any(|field| field.is_empty()) {
        return Err(module_name_conflict(module_name));
    }
    Ok(fields)
}

fn module_name_conflict(module_name: &[u8]) -> String {
    format!(
        "name conflict for module '{}'",
        String::from_utf8_lossy(module_name)
    )
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

fn table_string_field(l: &LuaState, table: GcRef<Table>, key: &str) -> Option<String> {
    let Value::String(value) = crate::registration::find_table_field(l, table, key.as_bytes())
        .ok()
        .flatten()?
    else {
        return None;
    };
    l.with_string_bytes(value, |bytes| {
        std::str::from_utf8(bytes).ok().map(str::to_owned)
    })
    .ok()
    .flatten()
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
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    key: &Value,
    value: &Value,
) {
    if table.is_null() {
        return;
    }
    let Value::String(key) = key else {
        panic!("package publication only supports string table keys");
    };
    gc.with_publication(|transaction| {
        let table = transaction
            .protect(table)
            .expect("package target Table must remain collector-valid");
        let key = transaction
            .protect(*key)
            .expect("package key must remain collector-valid");
        transaction
            .set_table_value(&table, &key, value)
            .expect("package table edge must remain collector-valid");
    });
}

fn global_value(l: &LuaState, name: &str) -> Option<Value> {
    let global = l.global_table?;
    crate::registration::find_table_field(l, global, name.as_bytes())
        .ok()
        .flatten()
}

fn push_lua_string(l: &mut LuaState, text: &str) -> bool {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        return false;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    crate::registration::push_string(l, gc, text.as_bytes()).is_ok()
}

fn value_to_string(l: &LuaState, value: Value) -> String {
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
        Value::String(s) => l
            .with_string_bytes(s, |bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default(),
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
        let _ = crate::registration::push_string(l, gc, err.message.as_bytes());
    }
}

fn raise_string(l: &mut LuaState, message: &str) -> i32 {
    let Some(gc_ptr) = l.active_gc_ptr() else {
        l.push_value(Value::Nil);
        return -1;
    };
    // SAFETY: LuaState::gc is installed by the VM before calling C functions.
    let gc = unsafe { &mut *gc_ptr };
    let _ = crate::registration::push_string(l, gc, message.as_bytes());
    -1
}

use lua_app::{execute, AppError};
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn temp_directory(label: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "lua_rust_v17_{label}_{}_{}",
        std::process::id(),
        id
    ));
    std::fs::create_dir(&directory).unwrap();
    directory
}

fn lua_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn run(source: &str) -> Value {
    execute(source.as_bytes()).unwrap()
}

#[test]
fn file_userdata_supports_binary_read_write_seek_close_and_type() {
    let directory = temp_directory("file");
    let path = directory.join("binary.dat");
    let source = format!(
        "local path = '{}'\n\
         local file, message = io.open(path, 'w+')\n\
         assert(file and message == nil and io.type(file) == 'file')\n\
         assert(file:write(string.char(65, 0, 255), 12.5) == file)\n\
         assert(file:flush())\n\
         assert(file:seek('set', 0) == 0)\n\
         local bytes = file:read('*a')\n\
         assert(#bytes == 7 and string.byte(bytes, 1, 3) == 65)\n\
         assert(file:close() and io.type(file) == 'closed file')\n\
         local ok = pcall(function() file:write('x') end)\n\
         local removed = os.remove(path)\n\
         return not ok and removed and io.type({{}}) == nil",
        lua_path(&path)
    );
    assert_eq!(run(&source), Value::Boolean(true));
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn file_read_formats_handle_numbers_lines_counts_and_eof() {
    let directory = temp_directory("formats");
    let path = directory.join("formats.txt");
    std::fs::write(&path, b"12.5\r\nalpha\nXYZ").unwrap();
    let source = format!(
        "local file = assert(io.open('{}', 'r'))\n\
         local number, rest, line = file:read('*n', '*l', '*l')\n\
         local bytes = file:read(2)\n\
         local tail = file:read('*a')\n\
         local eof = file:read(1)\n\
         file:close()\n\
         return number == 12.5 and rest == '' and line == 'alpha'\n\
           and bytes == 'XY' and tail == 'Z' and eof == nil",
        lua_path(&path)
    );
    assert_eq!(run(&source), Value::Boolean(true));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn garbage_collection_drops_unreachable_open_files() {
    let directory = temp_directory("gc_file");
    let path = directory.join("owned.txt");
    let source = format!(
        "local path = '{}'\n\
         do\n\
           local file = assert(io.open(path, 'w'))\n\
           file:write('owned')\n\
         end\n\
         collectgarbage('collect')\n\
         return os.remove(path)",
        lua_path(&path)
    );
    assert_eq!(run(&source), Value::Boolean(true));
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn vm_shutdown_drains_still_open_host_files() {
    let directory = temp_directory("shutdown_file");
    let path = directory.join("shutdown.txt");
    let proto = compile(
        format!(
            "shutdown_file = assert(io.open('{}', 'w'))\nshutdown_file:write('owned')\nreturn true",
            lua_path(&path)
        )
        .as_bytes(),
    )
    .unwrap();
    {
        let mut vm = Vm::new();
        assert_eq!(vm.execute(&proto).unwrap(), Value::Boolean(true));
    }
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn os_file_operations_time_and_utc_date_have_platform_errors() {
    let directory = temp_directory("os");
    let from = directory.join("from.txt");
    let to = directory.join("to.txt");
    std::fs::write(&from, b"move").unwrap();
    let source = format!(
        "local renamed, rename_error = os.rename('{}', '{}')\n\
         local removed, remove_error = os.remove('{}')\n\
         local missing, missing_error = os.remove('{}')\n\
         local epoch = os.date('!%Y-%m-%d %H:%M:%S', 0)\n\
         local parts = os.date('!*t', 0)\n\
         local bad_time = pcall(os.time, {{ year = 2000 }})\n\
         return renamed and rename_error == nil and removed and remove_error == nil\n\
           and not missing and type(missing_error) == 'string'\n\
           and epoch == '1970-01-01 00:00:00'\n\
           and parts.year == 1970 and parts.month == 1 and parts.day == 1\n\
           and parts.wday == 5 and os.difftime(10, 3) == 7\n\
           and os.clock() >= 0 and type(os.time()) == 'number'\n\
           and type(os.tmpname()) == 'string' and type(os.execute()) == 'number'\n\
           and not bad_time",
        lua_path(&from),
        lua_path(&to),
        lua_path(&to),
        lua_path(&directory.join("missing.txt")),
    );
    assert_eq!(run(&source), Value::Boolean(true));
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn loadstring_loadfile_and_dofile_share_the_compiler_pipeline() {
    let directory = temp_directory("loaders");
    let path = directory.join("chunk.lua");
    std::fs::write(&path, b"loaded_side_effect = 40\nreturn 2, 3").unwrap();
    let source = format!(
        "local dynamic = assert(loadstring('return 6 * 7'))\n\
         local broken, compile_error = loadstring('local =')\n\
         local file_loader, file_error = loadfile('{}')\n\
         local a, b = file_loader()\n\
         local c, d = dofile('{}')\n\
         local missing, missing_error = loadfile('{}')\n\
         return dynamic() == 42 and broken == nil and type(compile_error) == 'string'\n\
           and file_error == nil and a == 2 and b == 3 and c == 2 and d == 3\n\
           and loaded_side_effect == 40 and missing == nil and type(missing_error) == 'string'",
        lua_path(&path),
        lua_path(&path),
        lua_path(&directory.join("missing.lua")),
    );
    assert_eq!(run(&source), Value::Boolean(true));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn require_uses_preload_path_cache_and_cycle_sentinel() {
    let directory = temp_directory("modules");
    std::fs::write(
        directory.join("answer.lua"),
        b"module_loads = (module_loads or 0) + 1\nreturn { value = 42 }",
    )
    .unwrap();
    std::fs::write(
        directory.join("cycle_a.lua"),
        b"local b = require('cycle_b')\nreturn { b = b }",
    )
    .unwrap();
    std::fs::write(
        directory.join("cycle_b.lua"),
        b"local a = require('cycle_a')\nreturn { saw_sentinel = a == true }",
    )
    .unwrap();
    let source = format!(
        "package.path = '{}/?.lua'\n\
         package.preload.prebuilt = function(name) return {{ name = name }} end\n\
         local first = require('answer')\n\
         local second = require('answer')\n\
         local prebuilt = require('prebuilt')\n\
         local cycle = require('cycle_a')\n\
         local missing, message = pcall(require, 'missing')\n\
         return first == second and first.value == 42 and module_loads == 1\n\
           and package.loaded.answer == first and prebuilt.name == 'prebuilt'\n\
           and cycle.b.saw_sentinel and not missing and type(message) == 'string'",
        lua_path(&directory)
    );
    assert_eq!(run(&source), Value::Boolean(true));
    for name in ["answer.lua", "cycle_a.lua", "cycle_b.lua"] {
        std::fs::remove_file(directory.join(name)).unwrap();
    }
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn failed_require_rolls_back_cache_and_can_be_retried() {
    let directory = temp_directory("retry");
    let path = directory.join("retry.lua");
    std::fs::write(&path, b"error('first failure')").unwrap();
    let first = compile(
        format!(
            "package.path = '{}/?.lua'\nlocal ok = pcall(require, 'retry')\nreturn ok, package.loaded.retry",
            lua_path(&directory)
        )
        .as_bytes(),
    )
    .unwrap();
    let mut vm = Vm::new();
    assert_eq!(
        vm.execute_multi(&first).unwrap(),
        vec![Value::Boolean(false), Value::Nil]
    );
    std::fs::write(&path, b"return 42").unwrap();
    let second = compile(b"return require('retry')").unwrap();
    assert_eq!(vm.execute(&second).unwrap(), Value::Number(42.0));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn debug_library_inspects_metatables_upvalues_registry_and_traceback() {
    assert_eq!(
        run("local captured = 40\n\
             local function closure() return captured end\n\
             local name, old = debug.getupvalue(closure, 1)\n\
             local changed = debug.setupvalue(closure, 1, 42)\n\
             local info = debug.getinfo(closure)\n\
             local current = debug.getinfo(0)\n\
             local item = {}\n\
             local mt = { marker = true }\n\
             debug.setmetatable(item, mt)\n\
             local trace = debug.traceback('marker')\n\
             local registry = debug.getregistry()\n\
             registry.answer = 42\n\
             local bad = pcall(debug.getupvalue, true, 1)\n\
             return name == 'captured' and old == 40 and changed == 'captured'\n\
               and closure() == 42 and info.what == 'Lua' and type(info.source) == 'string'\n\
               and current.what == 'Lua' and debug.getmetatable(item) == mt\n\
               and string.find(trace, 'stack traceback:', 1, true) ~= nil\n\
               and debug.getregistry().answer == 42 and not bad"),
        Value::Boolean(true)
    );
}

#[test]
fn io_open_reports_platform_failure_without_panicking() {
    let result = execute(b"local file, message = io.open('missing/path/file', 'r')\nreturn file == nil and type(message) == 'string'").unwrap();
    assert_eq!(result, Value::Boolean(true));

    let error = execute(b"return io.open('x', 'invalid')").unwrap_err();
    assert!(matches!(error, AppError::Runtime(_)));
}

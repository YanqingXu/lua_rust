use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::Runtime;
use lua_vm::execute::execute_proto;
use std::path::{Path, PathBuf};

fn compile_and_run(source: &str) -> Runtime {
    let mut runtime = Runtime::new();
    {
        let mut parts = runtime.parts_mut().expect("runtime parts are available");
        let (state, gc, string_pool) = parts.split_mut();
        open_all(state, gc);

        let mut parser = Parser::new(source);
        let chunk = parser.parse().expect("parse should succeed");
        let cg = CodeGenerator::new_with_pool(gc, string_pool);
        let proto = cg
            .generate(&chunk, "<io-byte-test>")
            .expect("codegen should succeed");
        let proto = gc.create(proto);
        execute_proto(state, proto, gc).expect("VM should execute");
    }
    runtime
}

fn returned_bytes(runtime: &mut Runtime) -> Vec<u8> {
    let mut parts = runtime.parts_mut().expect("runtime parts remain available");
    let (state, _, _) = parts.split_mut();
    match state.stack.at(0).cloned().unwrap_or(Value::Nil) {
        Value::String(value) => state
            .copy_string_bytes(value)
            .expect("returned string should be live"),
        value => panic!("expected returned string, got {value:?}"),
    }
}

fn returned_number(runtime: &Runtime) -> f64 {
    match runtime
        .main_state()
        .and_then(|state| state.stack.at(0))
        .cloned()
        .unwrap_or(Value::Nil)
    {
        Value::Number(value) => value,
        value => panic!("expected returned number, got {value:?}"),
    }
}

fn unique_temp_path(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "lua_rust_io_bytes_{label}_{}_{}",
        std::process::id(),
        stamp
    ))
}

fn lua_path_literal(path: &Path) -> String {
    let path = path
        .to_str()
        .expect("the generated host path should be valid UTF-8")
        .replace('\\', "/");
    format!("\"{}\"", path.replace('"', "\\\""))
}

#[test]
fn tmpfile_round_trips_nul_and_high_bytes_with_byte_positions_and_lines() {
    let mut runtime = compile_and_run(
        r#"
        local raw = string.char(0, 128, 255, 10, 255, 0)
        local file = assert(io.tmpfile())
        assert(file:write(raw))
        assert(file:seek("cur", 0) == 6)
        assert(file:seek("set", 0) == 0)
        local first = file:read("*l")
        assert(#first == 3)
        assert(string.byte(first, 1) == 0)
        assert(string.byte(first, 2) == 128)
        assert(string.byte(first, 3) == 255)
        assert(file:seek("cur", 0) == 4)
        local second = file:read("*l")
        assert(#second == 2)
        assert(string.byte(second, 1) == 255)
        assert(string.byte(second, 2) == 0)
        assert(file:seek("set", 0) == 0)
        local lines = file:lines()
        assert(lines() == first)
        assert(lines() == second)
        assert(lines() == nil)
        assert(file:seek("cur", 0) == 6)
        assert(file:seek("end", -2) == 4)
        return file:read(2)
        "#,
    );

    assert_eq!(returned_bytes(&mut runtime), [0xff, 0x00]);
}

#[test]
fn real_file_direct_write_and_read_preserve_raw_bytes_and_offsets() {
    let path = unique_temp_path("roundtrip.bin");
    let source = format!(
        r#"
        local path = {path}
        local raw = string.char(0, 128, 255, 10, 255, 0)
        local output = assert(io.open(path, "wb"))
        assert(output:write(raw))
        assert(output:seek("cur", 0) == 6)
        assert(output:close())

        local input = assert(io.open(path, "rb"))
        local first = input:read("*l")
        assert(#first == 3)
        assert(string.byte(first, 1) == 0)
        assert(string.byte(first, 2) == 128)
        assert(string.byte(first, 3) == 255)
        assert(input:seek("cur", 0) == 4)
        assert(input:seek("end", -2) == 4)
        return input:read(2)
        "#,
        path = lua_path_literal(&path),
    );

    let mut runtime = compile_and_run(&source);
    assert_eq!(returned_bytes(&mut runtime), [0xff, 0x00]);
    assert_eq!(
        std::fs::read(&path).expect("raw output file should be readable"),
        [0x00, 0x80, 0xff, 0x0a, 0xff, 0x00]
    );
    std::fs::remove_file(path).expect("raw output file should be removable");
}

#[test]
fn invalid_utf8_host_path_returns_stable_nil_error_tuple() {
    let runtime = compile_and_run(
        r#"
        local file, message, code = io.open(string.char(255), "rb")
        if file == nil
            and message == "file path must be valid UTF-8"
            and code == 0
        then
            return 1
        end
        return 0
        "#,
    );

    assert_eq!(returned_number(&runtime), 1.0);
}

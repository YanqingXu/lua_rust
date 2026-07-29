use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

// The fixed lua_cpp oracle exposes this project-qualified value instead of the
// stock Lua 5.1 `_VERSION` string.
const CPP_ORACLE_VERSION: &str = "Lua 5.1 (C core prototype)";

fn run_lua(source: &str, stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", source])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lua_app should start");

    let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
    child_stdin
        .write_all(stdin)
        .expect("test input should be written");
    drop(child_stdin);

    child
        .wait_with_output()
        .expect("lua_app should finish and expose its output")
}

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    path.push(format!(
        "lua_rust_observable_io_{name}_{}_{}",
        std::process::id(),
        stamp
    ));
    path
}

fn lua_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"")
}

fn run_lua_file(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should run the script file")
}

fn run_lua_file_with_args(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should run the script file with arguments")
}

#[test]
fn standard_output_error_and_cpp_oracle_version_are_process_visible() {
    let output = run_lua(
        "io.write('stdout|', _VERSION); io.flush(); io.stderr:write('stderr'); io.stderr:flush()",
        b"",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(
        output.stdout,
        format!("stdout|{CPP_ORACLE_VERSION}").as_bytes(),
        "io.write and _VERSION must be observable on the child stdout pipe"
    );
    assert_eq!(
        output.stderr, b"stderr",
        "io.stderr:write must be observable on the child stderr pipe"
    );
}

#[test]
fn standard_input_reads_from_the_child_stdin_pipe() {
    let output = run_lua(
        "local first = io.read('*l'); local second = io.stdin:read('*l'); io.write(first, '|', second)",
        b"alpha\nbeta\n",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"alpha|beta");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn standard_input_and_output_preserve_arbitrary_lua_bytes() {
    let input = [0x00, 0x80, 0xff, b'\n'];
    let output = run_lua("local bytes = io.read(4); io.write(bytes)", &input);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, input);
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn print_writes_lua_string_bytes_without_utf8_reencoding() {
    let output = run_lua("print(string.char(0, 128, 255))", b"");

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, [0x00, 0x80, 0xff, b'\n']);
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn script_file_preserves_non_utf8_source_bytes() {
    let path = temp_path("non-utf8-source.lua");
    let mut source = b"print(string.byte(\"".to_vec();
    source.push(0xe1);
    source.extend_from_slice(b"\"))");
    std::fs::write(&path, source).expect("non-UTF-8 Lua source should be writable");

    let output = run_lua_file(&path);
    let _ = std::fs::remove_file(path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"225\n");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn script_argument_table_and_varargs_survive_runtime_publication_handoff() {
    let path = temp_path("published-args.lua");
    std::fs::write(
        &path,
        b"local first, second = ...; io.write(arg[0], '|', arg[1], '|', arg[2], '|', first, '|', second)",
    )
    .expect("argument fixture should be writable");

    let output = run_lua_file_with_args(&path, &["alpha", "beta"]);
    let _ = std::fs::remove_file(&path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    let expected = format!("{}|alpha|beta|alpha|beta", path.to_string_lossy());
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn xpcall_handler_failure_keeps_false_and_one_published_error_result() {
    let output = run_lua(
        "local ok, err = xpcall(\
             function() error('primary failure') end,\
             function() error('handler failure') end\
         ); \
         assert(ok == false); \
         assert(type(err) == 'string' and string.find(err, 'handler failure')); \
         io.write('protected-error-ok')",
        b"",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"protected-error-ok");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn command_line_rejects_legacy_pseudo_dump_files() {
    let path = temp_path("legacy-pseudo-dump.lua");
    std::fs::write(
        &path,
        b"\x1bLuaRustDump:999:696f2e777269746528276c65676163792729",
    )
    .expect("legacy pseudo-dump fixture should be writable");

    let output = run_lua_file(&path);
    let _ = std::fs::remove_file(path);

    assert!(
        !output.status.success(),
        "legacy pseudo-dump unexpectedly executed: {output:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "legacy pseudo-dump source fallback unexpectedly ran: {output:?}"
    );
    assert!(
        !output.stderr.is_empty(),
        "syntax rejection should be visible on stderr: {output:?}"
    );
}

#[test]
fn input_and_output_redirection_preserve_the_standard_handles() {
    let input_path = temp_path("input.txt");
    let output_path = temp_path("output.txt");
    std::fs::write(&input_path, b"from-file\n")
        .expect("redirected input fixture should be written");

    let source = format!(
        "io.input(\"{input}\"); local file_value = io.read('*l'); \
         io.input(io.stdin); local stdin_value = io.read('*l'); \
         io.output(\"{output}\"); io.write(file_value); io.flush(); \
         io.stdout:write(stdin_value); io.stdout:flush(); \
         io.output(io.stdout); io.write('|tail')",
        input = lua_path(&input_path),
        output = lua_path(&output_path),
    );
    let output = run_lua(&source, b"from-stdin\n");
    let redirected = std::fs::read(&output_path).expect("redirected output should exist");
    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(redirected, b"from-file");
    assert_eq!(output.stdout, b"from-stdin|tail");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

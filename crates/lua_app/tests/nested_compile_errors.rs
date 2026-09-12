//! Nested codegen failures must reject the whole chunk before execution.
//! Exact diagnostic contracts follow the pinned project C++ CLI. Stock Lua
//! rejects the same source during parsing and has different error wording.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
const INVALID_SOURCE: &[u8] = b"nested_compile_side_effect = (nested_compile_side_effect or 0) + 1; io.write(\"invalid chunk executed\\n\"); do 1=2 end";

struct ScratchDirectory {
    root: PathBuf,
    path: PathBuf,
}

impl ScratchDirectory {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should exist");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        let path = root.join(format!(
            "lua-nested-compile-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(path.is_absolute() && path.parent() == Some(root.as_path()));
        fs::create_dir(&path).expect("isolated CLI directory should be created");
        let path = path
            .canonicalize()
            .expect("owned CLI directory should resolve");
        assert_eq!(path.parent(), Some(root.as_path()));
        Self { root, path }
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        // Resolve again before recursive removal. A substituted path must not
        // escape the exact, uniquely created child of the canonical temp root.
        if let Ok(resolved) = self.path.canonicalize()
            && resolved == self.path
            && resolved.is_absolute()
            && resolved.parent() == Some(self.root.as_path())
        {
            let _ = fs::remove_dir_all(resolved);
        }
    }
}

struct ReapOnDrop(Child);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_cli(args: &[&str], input: &[u8], files: &[(&str, &[u8])]) -> Output {
    let scratch = ScratchDirectory::new();
    for (name, bytes) in files {
        fs::write(scratch.path.join(name), bytes).expect("test fixture should be written");
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_lua_app"));
    command
        .args(args)
        .current_dir(&scratch.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("LUA_INIT")
        {
            command.env_remove(name);
        }
    }
    let mut child = ReapOnDrop(command.spawn().expect("CLI should start"));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stdin = child.0.stdin.take().expect("stdin should be piped");
    let mut stdout = child.0.stdout.take().expect("stdout should be piped");
    let mut stderr = child.0.stderr.take().expect("stderr should be piped");
    let input = input.to_vec();
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().expect("child status should be readable") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.0.kill().expect("timed-out CLI should stop");
            break (
                child.0.wait().expect("timed-out CLI should be reaped"),
                true,
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = Output {
        status,
        stdout: stdout_reader
            .join()
            .expect("stdout reader should finish")
            .expect("stdout should be captured"),
        stderr: stderr_reader
            .join()
            .expect("stderr reader should finish")
            .expect("stderr should be captured"),
    };
    let write_result = writer.join().expect("stdin writer should finish");
    assert!(!timed_out, "CLI exceeded 30 seconds: {output:?}");
    write_result.expect("all input bytes should be delivered");
    output
}

fn host_text(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

fn assert_output(output: Output, status: i32, stdout: &str, stderr: &str) {
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert_eq!(output.stdout, host_text(stdout), "stdout: {output:?}");
    assert_eq!(output.stderr, host_text(stderr), "stderr: {output:?}");
}

fn cli_codegen_error() -> String {
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("CLI executable should have a name")
        .to_str()
        .expect("CLI executable name should be UTF-8");
    format!("{executable}: loadstring: Expression is not a valid lvalue\n")
}

#[test]
fn nested_structures_reject_invalid_lvalues_and_preserve_the_first_error() {
    assert_output(
        run_cli(
            &[
                "-e",
                include_str!("../../../tests/characterization/nested-compile-errors.lua"),
            ],
            b"",
            &[],
        ),
        0,
        "nested compilation contracts passed\n",
        "",
    );
}

#[test]
fn reader_and_file_loaders_reject_before_execution_then_recover() {
    let source = r#"
nested_compile_side_effect = 0
local sent = false
local bad, diagnostic = load(function()
  if sent then return nil end
  sent = true
  return "nested_compile_side_effect = 1; if false then 1=2 end"
end, "=nested-reader")
assert(bad == nil, "reader unexpectedly returned an executable function")
assert(diagnostic == "load: Expression is not a valid lvalue", diagnostic)
bad, diagnostic = loadfile("invalid-nested.lua")
assert(bad == nil, "file loader unexpectedly returned an executable function")
assert(diagnostic == "loadfile: invalid-nested.lua: Expression is not a valid lvalue", diagnostic)
local ok, error_object = pcall(dofile, "invalid-nested.lua")
assert(ok == false, "dofile accepted an invalid chunk")
assert(error_object == "loadfile: invalid-nested.lua: Expression is not a valid lvalue", error_object)
assert(nested_compile_side_effect == 0, "invalid chunk executed before rejection")
collectgarbage("collect")
local good, error_message = loadstring("local x=40; return function() return x+2 end")
assert(good, error_message)
local closure = good()
good = nil
collectgarbage("collect")
assert(closure() == 42)
print("loader rejection contracts passed")
"#;
    assert_output(
        run_cli(
            &["-e", source],
            b"",
            &[("invalid-nested.lua", INVALID_SOURCE)],
        ),
        0,
        "loader rejection contracts passed\n",
        "",
    );
}

#[test]
fn batch_compile_failure_prevents_chunk_and_later_startup_side_effects() {
    assert_output(
        run_cli(
            &[
                "-e",
                std::str::from_utf8(INVALID_SOURCE).unwrap(),
                "-e",
                "print('later startup executed')",
            ],
            b"",
            &[],
        ),
        1,
        "",
        &cli_codegen_error(),
    );
}

#[test]
fn script_compile_failure_prevents_all_chunk_side_effects() {
    assert_output(
        run_cli(
            &["invalid-nested.lua"],
            b"",
            &[("invalid-nested.lua", INVALID_SOURCE)],
        ),
        1,
        "",
        &cli_codegen_error(),
    );
}

#[test]
fn repl_rejects_the_whole_chunk_and_resumes_with_clean_state() {
    let mut input = b"nested_compile_side_effect = 0\n".to_vec();
    input.extend_from_slice(INVALID_SOURCE);
    input.extend_from_slice(
        b"\n=nested_compile_side_effect\n=6*7\ncollectgarbage('collect'); print('after')\n",
    );
    assert_output(
        run_cli(&["-e", "_PROMPT='' _PROMPT2=''", "-i"], &input, &[]),
        0,
        "0\n42\nafter\n\n",
        "Expression is not a valid lvalue\n",
    );
}

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn run(arguments: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_bytecode"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("lua_bytecode timed out: {arguments:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn all_view_formats_have_stable_top_level_contracts() {
    let source = b"local answer = 40 + 2\nreturn answer";
    let text = run(&["--format", "text", "-"], source);
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("Add"));

    let json = run(&["--format=json", "-"], source);
    assert!(json.status.success());
    let json_text = String::from_utf8_lossy(&json.stdout);
    assert!(json_text.starts_with("{\"schema\":\"lua-rust-bytecode/v1\""));
    assert!(json_text.contains("\"code\":["));

    let cfg = run(&["--format", "cfg", "-"], source);
    assert!(cfg.status.success());
    assert!(String::from_utf8_lossy(&cfg.stdout).starts_with("digraph bytecode"));

    let trace = run(&["--format", "trace", "-"], source);
    assert!(trace.status.success());
    let trace_text = String::from_utf8_lossy(&trace.stdout);
    assert!(trace_text.starts_with("{\"schema\":\"lua-rust-vm-trace/v1\""));
    assert!(trace_text.contains("\"op\":\"Add\""));
}

#[test]
fn opcode_audit_is_a_standalone_versioned_contract() {
    let output = run(&["--format", "opcode-audit"], b"");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("{\"schema\":\"lua-rust-lua51-opcode-audit/v1\""));
    assert!(stdout.contains("\"binary_chunk_compatible\":false"));
    assert_eq!(stdout.matches("\"ordinal\":").count(), 38);
}

#[test]
fn tool_usage_compile_and_runtime_errors_are_separate() {
    let usage = run(&["--format", "unknown", "-"], b"return 1");
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("unknown format"));

    let compile = run(&["--format", "json", "-"], b"local =");
    assert_eq!(compile.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&compile.stderr).contains("expected"));

    let runtime = run(&["--format", "trace", "-"], b"error('trace failure')");
    assert_eq!(runtime.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&runtime.stderr).contains("trace failure"));
}

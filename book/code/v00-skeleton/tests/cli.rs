use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn stdin_reaches_the_interpreter_boundary() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua-step"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"print('hello')")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "chunk=stdin bytes=14\n"
    );
}

#[test]
fn missing_file_is_a_stderr_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua-step"))
        .arg("definitely-missing-v00.lua")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("definitely-missing-v00.lua"));
}

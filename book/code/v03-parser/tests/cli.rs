use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lua-step"))
}

#[test]
fn demo_prints_ast_and_result() {
    let output = Command::new(binary())
        .arg("examples/demo.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("op: Add"));
    assert!(stdout.contains("op: Multiply"));
    assert!(stdout.contains("result=7"));
}

#[test]
fn malformed_expression_has_nonzero_exit_code() {
    let output = Command::new(binary())
        .arg("examples/invalid.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected an expression"));
}

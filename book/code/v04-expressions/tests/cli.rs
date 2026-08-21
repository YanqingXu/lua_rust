use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lua-step"))
}

#[test]
fn demo_exercises_short_circuit_and_concat() {
    let output = Command::new(binary())
        .arg("examples/demo.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "type=string value=\"answer=42\"\n"
    );
}

#[test]
fn type_error_uses_stderr_and_exit_two() {
    let output = Command::new(binary())
        .arg("examples/invalid.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("arithmetic needs numbers"));
}

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lua-step"))
}

#[test]
fn demo_prints_a_positioned_token_stream() {
    let output = Command::new(binary())
        .arg("examples/demo.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("2:1\tlocal"));
    assert!(stdout.contains("2:7\tname(message)"));
    assert!(stdout.contains("string(\"hello\\nLua\")"));
    assert!(stdout.ends_with("<eof>\n"));
}

#[test]
fn invalid_byte_is_reported_on_stderr() {
    let output = Command::new(binary())
        .arg("examples/invalid.lua")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected byte 0x40"));
}

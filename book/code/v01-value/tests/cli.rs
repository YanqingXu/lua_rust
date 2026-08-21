use std::process::Command;

#[test]
fn demo_literal_has_type_and_value() {
    let script = format!("{}/examples/demo.lua", env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO_BIN_EXE_lua-step"))
        .arg(script)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "type=string value=\"Lua bytes: \\x00\\xff\"\n"
    );
}

#[test]
fn unsupported_input_is_an_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua-step"))
        .arg(format!(
            "{}/examples/invalid.lua",
            env!("CARGO_MANIFEST_DIR")
        ))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported literal"));
}

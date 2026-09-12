use std::process::Command;

#[test]
fn coroutine_tracebacks_render_each_retained_frame_once() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/debug-coroutine-traceback.lua"),
        ])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "debug coroutine traceback passed"
    );
}

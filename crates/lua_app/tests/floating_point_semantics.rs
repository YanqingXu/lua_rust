use std::process::Command;

#[test]
fn floating_point_matrix_matches_the_two_lua_oracles() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/floating-point-matrix.lua"),
        ])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "floating-point matrix passed"
    );
}

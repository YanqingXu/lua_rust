use std::process::Command;

#[test]
fn audited_string_failures_match_both_oracles_error_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/string-common-error-contract.lua"),
        ])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "string common error contract passed"
    );
}

#[test]
fn string_argument_errors_preserve_error_values_and_the_host_process() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/string-argument-boundary.lua"),
            "-e",
            "print('next script passed')",
        ])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["string argument boundary passed", "next script passed"]
    );
}

#[test]
fn oversized_format_fields_and_malformed_patterns_are_controlled_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            r#"
            for _, format in ipairs({'%18446744073709551615s', '%.18446744073709551615f'}) do
                local ok, err = pcall(string.format, format, 1)
                assert(not ok and type(err) == 'string')
            end
            for _, pattern in ipairs({'[', '%', '%b', '%f', '(', '%1'}) do
                local ok, err = pcall(string.find, 'abc', pattern)
                assert(not ok and type(err) == 'string')
            end
            print('controlled errors passed')
            "#,
            "-e",
            "print('next script passed')",
        ])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["controlled errors passed", "next script passed"]
    );
}

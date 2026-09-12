use std::process::Command;

fn assert_survives(source: &str, expected: &str) {
    // A process boundary is intentional: these inputs previously aborted at
    // extern "C", which cannot be caught by Rust's or Lua's protected calls.
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", source, "-e", "print('next script passed')"])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("test output is UTF-8");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [expected, "next script passed"]
    );
}

#[test]
fn rawset_errors_are_catchable_and_leave_the_process_usable() {
    assert_survives(
        include_str!("../../../tests/characterization/rawset-error-boundary.lua"),
        "rawset error boundary passed",
    );
}

#[test]
fn os_time_invalid_fields_are_catchable_and_leave_the_process_usable() {
    assert_survives(
        include_str!("../../../tests/characterization/os-time-error-boundary.lua"),
        "os.time error boundary passed",
    );
}

#[test]
fn next_rejects_invalid_cursors_with_a_lua_error() {
    assert_survives(
        r#"
        for _, key in ipairs({'missing', 100, math.sqrt(-1)}) do
            local ok, err = pcall(next, {}, key)
            assert(not ok and type(err) == 'string')
            local message = "invalid key to 'next'"
            assert(string.sub(err, -#message) == message, err)
        end
        local t = {a = 1, b = 2, c = 3}
        local first = next(t)
        local second = next(t, first)
        t[second] = nil
        local third = next(t, second)
        assert(third ~= first and third ~= second and third ~= nil)
        print('next error boundary passed')
        "#,
        "next error boundary passed",
    );
}

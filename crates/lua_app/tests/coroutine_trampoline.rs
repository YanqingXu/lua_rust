use std::path::PathBuf;
use std::process::Command;

fn run_lua(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(args)
        .output()
        .expect("lua_app should start")
}

#[test]
fn normal_ancestor_reentry_matches_the_fixed_cpp_oracle() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/characterization/coroutine-normal-ancestor.lua");
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(fixture)
        .output()
        .expect("lua_app should execute the characterization fixture");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("oracle output is UTF-8"),
        concat!(
            "1\tmain:before\tsuspended\tsuspended\t-\t-\n",
            "2\tA:enter\tfrom-main\trunning\tsuspended\t-\n",
            "3\tB:enter\tfrom-A\tnormal\trunning\t-\n",
            "4\tA:after-B\tfrom-B\tnil\trunning\tnormal\n",
            "5\tB:after-A\ttrue\tA-done\tdead\trunning\n",
            "6\tA:after-B\tnil\tnil\trunning\tdead\n",
            "7\tmain:after-A\tfalse\t<resume-error>\tdead\tdead\n",
        )
    );
}

#[test]
fn protected_resume_crosses_the_c_boundary_without_recursive_state_borrowing() {
    let output = run_lua(&[
        "-e",
        concat!(
            "local co = coroutine.create(function(value) ",
            "  coroutine.yield(value + 1); error('boom') ",
            "end); ",
            "print(pcall(coroutine.resume, co, 40)); ",
            "print(pcall(coroutine.resume, co))"
        ),
    ]);

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("test output is UTF-8");
    assert_eq!(stdout.lines().next(), Some("true\ttrue\t41"));
    let second = stdout.lines().nth(1).expect("second pcall result");
    assert!(second.starts_with("true\tfalse\t"));
    assert!(second.ends_with(":1: boom"), "{second}");
}

#[test]
fn wrap_transfers_yield_values_and_propagates_error_identity() {
    let output = run_lua(&[
        "-e",
        concat!(
            "local wrapped = coroutine.wrap(function(value) ",
            "  local resumed = coroutine.yield(value * 2); error(resumed) ",
            "end); ",
            "print(wrapped(6)); ",
            "local ok, err = pcall(wrapped, 'wrapped-error'); ",
            "print(ok, err)"
        ),
    ]);

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("test output is UTF-8");
    assert_eq!(stdout.lines().next(), Some("12"));
    let second = stdout.lines().nth(1).expect("wrap error result");
    assert!(second.starts_with("false\t"), "{second}");
    assert!(second.ends_with(":1: wrapped-error"), "{second}");
}

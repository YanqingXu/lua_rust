use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_lua(source: &str, expected: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", source])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
}

#[test]
fn protected_errors_capture_origins_without_changing_resume_policy() {
    run_lua(
        include_str!("../../../tests/characterization/error-provenance.lua"),
        "error provenance passed",
    );
}

#[test]
fn explicit_objects_and_branch_sensitive_names_match_both_oracles() {
    run_lua(
        include_str!("../../../tests/characterization/error-object-and-names.lua"),
        "error object and names passed",
    );
}

#[test]
fn unprotected_arithmetic_reports_its_captured_source() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", "local t={}\nreturn t+1"])
        .output()
        .expect("lua_app should start");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(
            "(command line):2: attempt to perform arithmetic on local 't' (a table value)"
        ),
        "{stderr}"
    );
}

#[test]
fn recursive_lua_error_does_not_abort_the_host_process() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/recursive-error-boundary.lua"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("isolated recursive CLI test should start");
    let deadline = Instant::now() + Duration::from_secs(30);
    let timed_out = loop {
        if child
            .try_wait()
            .expect("child status should be readable")
            .is_some()
        {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().expect("timed-out child should stop");
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child
        .wait_with_output()
        .expect("child output should be captured");
    assert!(!timed_out, "recursive CLI timed out: {output:?}");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "recursive error boundary continued"
    );
}

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn debug_hooks_preserve_frame_history_and_count_line_order() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            include_str!("../../../tests/characterization/debug-line-hooks.lua"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("isolated debug-hook CLI should start");
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
    assert!(!timed_out, "debug-hook CLI timed out: {output:?}");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "debug line hooks passed"
    );
}

use std::path::Path;

use super::{
    model::{self, SendInput, StartDrains},
    solution,
};
use crate::protocol;

fn exact(label: &str, actual: &[u8], expected: &[u8]) {
    assert!(
        actual == expected,
        "{label}: byte mismatch (actual={}, expected={}, first_diff={:?})",
        actual.len(),
        expected.len(),
        actual.iter().zip(expected).position(|(a, b)| a != b)
    );
}

// ANCHOR: runner_stdin_contract
pub fn sends_then_closes(binary: &Path, send: SendInput) {
    for input in [&b"hello\0\xff"[..], &b""[..]] {
        let report = model::run(binary, "stdin", input, send, solution::start_drains).unwrap();
        assert!(
            report.ready && report.ack,
            "child must acknowledge its first read"
        );
        assert!(
            !report.timed_out,
            "child received input but is still waiting for EOF"
        );
        assert!(report.status.success() && report.reaped && report.joined && !report.killed);
        exact("stdout", &report.stdout, &protocol::stdin_stdout(input));
        exact("stderr", &report.stderr, protocol::STDIN_EOF);
    }
}
// ANCHOR_END: runner_stdin_contract

// ANCHOR: runner_drains_contract
pub fn drains_both(binary: &Path, start: StartDrains) {
    let report = model::run(binary, "dual", b"G", solution::send_input, start).unwrap();
    assert!(
        report.ready && report.ack,
        "child consumed start token before flood"
    );
    assert!(
        !report.timed_out,
        "streams must drain while the child is still running"
    );
    assert!(report.status.success() && report.reaped && report.joined && !report.killed);
    exact("stdout", &report.stdout, &protocol::dual_stdout());
    exact("stderr", &report.stderr, &protocol::dual_stderr());
}
// ANCHOR_END: runner_drains_contract

mod contracts;
mod faults;
mod model;
#[path = "../runner_support/protocol.rs"]
mod protocol;
mod solution;

use std::path::Path;

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_runner_fixture"))
}

#[test]
fn stdin_send_then_close_delivers_eof() {
    contracts::sends_then_closes(binary(), solution::send_input);
}

#[test]
fn parallel_readers_capture_both_streams() {
    contracts::drains_both(binary(), solution::start_drains);
}

#[test]
fn retaining_stdin_blocks_after_first_byte_ack() {
    let report = model::run(
        binary(),
        "stdin",
        b"hello\0\xff",
        faults::retain_stdin,
        solution::start_drains,
    )
    .unwrap();
    assert!(
        report.ready
            && report.ack
            && report.timed_out
            && report.killed
            && report.reaped
            && report.joined
    );
    assert!(!report.status.success());
    assert_eq!(
        report.stdout,
        [protocol::STDIN_READY, protocol::STDIN_ACK].concat()
    );
    assert!(report.stderr.is_empty());
}

fn blocked_drain(start: model::StartDrains) {
    let report = model::run(binary(), "dual", b"G", solution::send_input, start).unwrap();
    assert!(
        report.ready
            && report.ack
            && report.timed_out
            && report.killed
            && report.reaped
            && report.joined
    );
    assert!(!report.status.success());
    assert_eq!(
        report.stdout,
        [protocol::DUAL_READY, protocol::DUAL_ACK].concat()
    );
    let expected = protocol::dual_stderr();
    // A killed synchronous write may leave no committed bytes on Windows.
    assert!(report.stderr.len() < expected.len());
    assert!(
        expected.starts_with(&report.stderr),
        "partial stderr must be an exact prefix"
    );
}

#[test]
fn waiting_before_bulk_drain_blocks_after_dual_ack() {
    blocked_drain(faults::wait_before_bulk_drain);
}

#[test]
fn sequential_stdout_then_stderr_blocks_after_dual_ack() {
    blocked_drain(faults::stdout_then_stderr);
}

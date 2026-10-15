mod contracts;
mod model;
#[path = "../runner_support/protocol.rs"]
mod protocol;
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1: write stdin and close its last writer to deliver EOF"]
fn stdin_send_then_close_delivers_eof() {
    contracts::sends_then_closes(
        std::path::Path::new(env!("CARGO_BIN_EXE_runner_fixture")),
        starter::send_input,
    );
}

#[test]
#[ignore = "TODO 2: drain stdout and stderr concurrently while child is running"]
fn parallel_readers_capture_both_streams() {
    contracts::drains_both(
        std::path::Path::new(env!("CARGO_BIN_EXE_runner_fixture")),
        starter::start_drains,
    );
}

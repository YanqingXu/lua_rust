use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::mpsc::Sender;
use std::thread;

use super::model::{self, DrainTask, Notice};

// ANCHOR: runner_stdin_solution
pub fn send_input(stdin: ChildStdin, input: &[u8]) -> Result<Option<ChildStdin>, String> {
    let stdin = model::write_flush(stdin, input)?;
    drop(stdin); // Closing the last parent writer delivers EOF after the bytes.
    Ok(None)
}
// ANCHOR_END: runner_stdin_solution

// ANCHOR: runner_drains_solution
pub fn start_drains(
    stdout: ChildStdout,
    stderr: ChildStderr,
    notices: Sender<Notice>,
) -> DrainTask {
    let output = thread::spawn(move || model::read_pipe(stdout, Some(notices)));
    let error = thread::spawn(move || model::read_pipe(stderr, None));
    DrainTask {
        worker: thread::spawn(move || model::join_pair(output, error)),
        after_wait: None,
    }
}
// ANCHOR_END: runner_drains_solution

use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::mpsc::Sender;

use super::model::{self, DrainTask, Notice};

// ANCHOR: runner_stdin_starter
pub fn send_input(stdin: ChildStdin, input: &[u8]) -> Result<Option<ChildStdin>, String> {
    // TODO 1: flush is not EOF; close the writer instead of retaining it.
    Ok(Some(model::write_flush(stdin, input)?))
}
// ANCHOR_END: runner_stdin_starter

// ANCHOR: runner_drains_starter
pub fn start_drains(
    stdout: ChildStdout,
    stderr: ChildStderr,
    notices: Sender<Notice>,
) -> DrainTask {
    // TODO 2: use two readers that can make progress independently.
    model::sequential(stdout, stderr, notices)
}
// ANCHOR_END: runner_drains_starter

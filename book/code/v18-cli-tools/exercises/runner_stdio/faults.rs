use std::io::Read;
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::mpsc::{self, Sender};
use std::thread;

use super::model::{self, DrainTask, Notice, Streams};

pub fn retain_stdin(stdin: ChildStdin, input: &[u8]) -> Result<Option<ChildStdin>, String> {
    Ok(Some(model::write_flush(stdin, input)?))
}

pub fn stdout_then_stderr(
    stdout: ChildStdout,
    stderr: ChildStderr,
    notices: Sender<Notice>,
) -> DrainTask {
    model::sequential(stdout, stderr, notices)
}

// ANCHOR: runner_wait_before_drain_fault
pub fn wait_before_bulk_drain(
    mut stdout: ChildStdout,
    stderr: ChildStderr,
    notices: Sender<Notice>,
) -> DrainTask {
    let (release, wait) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut prefix = Vec::new();
        for notice in [Notice::Ready, Notice::Ack] {
            loop {
                let mut byte = [0];
                stdout
                    .read_exact(&mut byte)
                    .map_err(|error| error.to_string())?;
                prefix.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let _ = notices.send(notice);
        }
        // Read only the handshake, then withhold bulk drain until wait ends.
        wait.recv().map_err(|error| error.to_string())?;
        let output = model::read_pipe(stdout, None);
        let error = model::read_pipe(stderr, None);
        prefix.extend(output?);
        Ok(Streams {
            stdout: prefix,
            stderr: error?,
        })
    });
    DrainTask {
        worker,
        after_wait: Some(release),
    }
}
// ANCHOR_END: runner_wait_before_drain_fault

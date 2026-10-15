use super::model::{CliError, Outcome, Transcript, USAGE};

// ANCHOR: cli_exit_solution
pub fn finish(outcome: Outcome) -> Transcript {
    let (code, stderr) = match outcome.result {
        Ok(_) => (0, Vec::new()),
        Err(CliError::Usage(message)) => (2, format!("lua: {message}\n{USAGE}\n").into_bytes()),
        Err(CliError::Runtime(message)) => (1, format!("lua: {message}\n").into_bytes()),
    };
    Transcript {
        code,
        stdout: outcome.stdout,
        stderr,
    }
}
// ANCHOR_END: cli_exit_solution

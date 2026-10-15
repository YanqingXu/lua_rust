use super::model::{error_transcript, Outcome, Transcript};

pub fn print_error_but_succeed(outcome: Outcome) -> Transcript {
    error_transcript(outcome, 0, 0)
}

pub fn all_errors_one(outcome: Outcome) -> Transcript {
    error_transcript(outcome, 1, 1)
}

// ANCHOR: cli_exit_print_return_fault
pub fn print_script_results(mut outcome: Outcome) -> Transcript {
    if let Ok(values) = &outcome.result {
        for value in values {
            outcome
                .stdout
                .extend_from_slice(format!("{value}\n").as_bytes());
        }
    }
    error_transcript(outcome, 2, 1)
}
// ANCHOR_END: cli_exit_print_return_fault

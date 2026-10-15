use super::model::{error_transcript, Outcome, Transcript};

// ANCHOR: cli_exit_starter
pub fn finish(outcome: Outcome) -> Transcript {
    // TODO: printing an error is not the same as returning a failure exit status.
    error_transcript(outcome, 0, 0)
}
// ANCHOR_END: cli_exit_starter

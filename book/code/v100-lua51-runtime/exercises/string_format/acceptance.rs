mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "implement the validated argument cursor TODO in starter.rs"]
fn argument_cursor_commits_only_after_validation() {
    contracts::argument_cursor_commits_only_after_validation(starter::consume);
}

mod contracts;
pub mod faults;
pub mod model;
mod starter;

#[test]
#[ignore = "实现 retry/starter.rs 的 attempt 后显式验收"]
fn failure_removes_only_the_current_loading_entry() {
    contracts::failed_entry_is_retryable(starter::attempt);
}

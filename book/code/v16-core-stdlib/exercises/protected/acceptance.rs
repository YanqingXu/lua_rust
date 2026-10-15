mod contracts;
pub mod faults;
mod model;
mod starter;

#[test]
#[ignore = "实现 protected/starter.rs 的 pack_pcall 后显式验收"]
fn pcall_keeps_the_complete_result_window() {
    contracts::complete_pcall_results(starter::pack_pcall);
}

#[test]
#[ignore = "实现 protected/starter.rs 的 pack_xpcall 后显式验收"]
fn xpcall_only_handles_failures_and_selects_one_result() {
    contracts::conditional_xpcall_handler(starter::pack_xpcall);
}

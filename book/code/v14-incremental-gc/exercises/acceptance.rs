mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO 1：starter.rs 的 split_step，显式验收"]
fn first_step_yields_before_collection_finishes() {
    contracts::bounded_first_step(starter::split_step);
}

#[test]
#[ignore = "TODO 2：starter.rs 的 resume_step，显式验收"]
fn saved_progress_finishes_the_same_cycle() {
    contracts::progress_survives_calls(starter::resume_step);
}

#[test]
#[ignore = "TODO 3：starter.rs 的 connect，显式验收"]
fn barrier_preserves_a_late_white_child() {
    contracts::late_edge_keeps_white_child(starter::connect);
}

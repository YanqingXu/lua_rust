mod contracts;
mod model;
#[allow(dead_code)] // Two healthy hooks are reused here; run_child is replaced by the learner.
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1: each coroutine owns its suspended continuation"]
fn suspended_frames_belong_to_each_coroutine() {
    contracts::per_coroutine_storage(starter::save_frame);
}

#[test]
#[ignore = "TODO 2: resume from the saved next pc"]
fn resume_does_not_restart_entry_side_effects() {
    contracts::resumes_after_yield(starter::resume_pc);
}

#[test]
#[ignore = "TODO 3: restore parent ownership and isolate yield channels"]
fn nested_owner_turn_restores_parent_and_channels() {
    contracts::restores_parent_turn(starter::run_child);
}

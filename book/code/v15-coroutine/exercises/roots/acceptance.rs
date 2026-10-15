mod contracts;
mod model;
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1：roots/starter.rs::mirror_roots，显式验收"]
fn suspended_values_are_mirrored_before_collection() {
    contracts::suspended_mirror_keeps_every_execution_value(starter::mirror_roots);
}

#[test]
#[ignore = "TODO 2：roots/starter.rs::extra_direct_roots，显式验收"]
fn unreachable_continuations_are_not_permanent_roots() {
    contracts::direct_roots_do_not_make_host_states_permanent(starter::extra_direct_roots);
}

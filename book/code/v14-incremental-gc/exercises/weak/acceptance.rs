mod contracts;
mod model;
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1: classify weak edges in weak/starter.rs"]
fn weak_edges_follow_table_mode() {
    contracts::classifies_weak_edges(starter::classify_edges);
}

#[test]
#[ignore = "TODO 2: reach the ephemeron fixed point in weak/starter.rs"]
fn ephemerons_reach_a_fixed_point_without_bootstrapping_cycles() {
    contracts::reaches_ephemeron_fixed_point(starter::propagate);
}

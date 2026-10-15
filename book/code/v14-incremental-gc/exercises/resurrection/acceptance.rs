mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "TODO：只修改 resurrection/starter.rs::finish_atomic，显式验收"]
fn pending_graph_is_marked_before_weak_cleanup() {
    contracts::pending_graph_precedes_weak_cleanup(starter::finish_atomic);
    contracts::no_rescue_dies_next_cycle(starter::finish_atomic);
}

pub mod contracts;
pub mod model;
mod starter;
pub mod temp;

#[test]
#[ignore = "TODO：file_prune/starter.rs::prune，显式验收"]
fn completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones() {
    contracts::completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones(starter::prune)
        .unwrap();
}

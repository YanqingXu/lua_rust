mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO：sort_roots/starter.rs::with_snapshot，显式验收"]
fn snapshot_roots_survive_callbacks_and_restore_on_error() {
    contracts::snapshot_lifetime_and_cleanup(starter::with_snapshot);
}

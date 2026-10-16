mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "implement complete capture-slot restoration in starter.rs"]
fn checkpoint_restore_replaces_old_slots_and_removes_new_slots() {
    contracts::checkpoint_restore_replaces_old_slots_and_removes_new_slots(starter::restore_slots);
}

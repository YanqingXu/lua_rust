mod contracts;
mod faults;
mod model;
mod real;
mod solution;
mod temp;

#[test]
fn completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones() {
    contracts::completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones(solution::prune)
        .unwrap();
}

#[test]
fn faulty_missing_prune_keeps_os_file_after_userdata_is_stale() {
    contracts::missing_prune(faults::forget_prune).unwrap();
}

#[test]
fn faulty_option_predicate_confuses_closed_and_collected() {
    contracts::option_is_not_liveness(faults::prune_closed_instead_of_dead).unwrap();
}

#[test]
fn faulty_rooting_all_file_keys_prevents_collection() {
    contracts::root_all_keys_leaks(solution::prune).unwrap();
}

#[test]
fn incomplete_sweep_keeps_map_until_completed_cycle() {
    contracts::incomplete_cycle_keeps_map(solution::prune).unwrap();
}

#[test]
fn real_vm_host_gc_clears_weak_slots_and_releases_os_handles() {
    real::vm_weak_slots_and_os_release().unwrap();
}

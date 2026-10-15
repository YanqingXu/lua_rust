mod contracts;
mod faults;
mod model;
mod real;
mod solution;

#[test]
fn pending_graph_is_marked_before_weak_cleanup() {
    contracts::pending_graph_precedes_weak_cleanup(solution::finish_atomic);
}

#[test]
fn faulty_early_cleanup_loses_entries_while_the_graph_still_survives() {
    let mut heap = model::ResurrectionHeap::default();
    let first = heap.cycle(faults::cleanup_too_early, true);
    assert_eq!(first.weak_before_callback, [false; 4]);
    assert_eq!(first.callback_saw_weak, Some([false; 4]));
    assert_eq!(first.live_after_sweep, [true; 4]);
    assert_eq!(
        (first.pending_before_callback, first.total_callbacks),
        (1, 1)
    );
    assert!(first.prefix_live);
    let second = heap.cycle(faults::cleanup_too_early, true);
    assert_eq!(second.weak_before_callback, [false; 4]);
    assert_eq!(second.live_after_sweep, [true; 4]);
    assert_eq!(second.total_callbacks, 1);
}

#[test]
fn without_rescue_the_second_cycle_reclaims_without_another_callback() {
    contracts::no_rescue_dies_next_cycle(solution::finish_atomic);
}

#[test]
fn real_heap_keeps_the_pending_graph_then_rescues_and_releases_it() {
    let cycles = real::heap_cycles(true);
    for cycle in &cycles[..2] {
        assert_eq!(cycle.weak, [true; 4]);
        assert_eq!(cycle.weak_nil, [false; 4]);
        assert_eq!(cycle.accessible, [true; 5]);
        assert_eq!(cycle.finalized, Some(true));
        assert_eq!(cycle.total_events, 1);
        assert!(cycle.prefix_live);
    }
    assert_eq!(cycles[2].weak, [false; 4]);
    assert_eq!(cycles[2].weak_nil, [true; 4]);
    assert_eq!(cycles[2].accessible, [false; 5]);
    assert_eq!(cycles[2].finalized, None);
    assert_eq!(cycles[2].total_events, 1);
    assert!(cycles[2].prefix_live);
}

#[test]
fn real_heap_without_rescue_has_second_death_in_the_next_cycle() {
    let cycles = real::heap_cycles(false);
    assert_eq!(cycles[0].weak, [true; 4]);
    assert_eq!(cycles[0].weak_nil, [false; 4]);
    assert_eq!(cycles[0].accessible, [true; 5]);
    assert_eq!(cycles[1].weak, [false; 4]);
    assert_eq!(cycles[1].weak_nil, [true; 4]);
    assert_eq!(cycles[1].accessible, [false; 5]);
    assert_eq!(cycles[1].total_events, 1);
    assert!(cycles.iter().all(|cycle| cycle.prefix_live));
}

#[test]
fn real_vm_callback_sees_weak_entries_and_rescue_lasts_until_released() {
    assert_eq!(
        real::vm_rescue_cycles(),
        std::array::from_fn(|_| lua_core::Value::Boolean(true))
    );
}

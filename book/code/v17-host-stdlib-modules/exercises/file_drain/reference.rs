mod contracts;
mod faults;
pub mod model;
mod real;
mod solution;
pub mod support;

#[test]
fn solution_drains_some_and_none_before_heap_shutdown() {
    contracts::drain_before_heap_shutdown(solution::shutdown);
}

#[test]
fn heap_only_keeps_files_alive_in_drop_body_but_fields_close_them_later() {
    let report = model::run(faults::heap_only).unwrap();
    let before = contracts::stage(&report, "heap.shutdown.begin");
    assert_eq!((before.entries, before.open_files), (3, 2));
    assert_eq!(before.drops, [1, 0, 0]);
    assert_eq!(report.reachable_before, [true; 3]);
    assert_eq!(report.released_objects, 4);
    assert_eq!(report.access_rejected_after_shutdown, [true; 3]);
    let exit = contracts::stage(&report, "drop.body.exit");
    assert_eq!((exit.heap_objects, exit.open_files), (0, 2));
    assert_eq!(exit.drops, [1, 0, 0]);
    assert_eq!(report.after_owner_drops, [1, 1, 1]);
}

#[test]
fn real_heap_shutdown_invalidates_reachable_userdata_and_discards_pending_finalizers() {
    assert_eq!(
        real::heap_shutdown_discards_pending_events().unwrap(),
        real::ShutdownReport {
            reachable_was_unfinalized: true,
            pending_was_finalized: true,
            released: 2,
            remaining: 0,
            access_rejected: [true; 2],
            events_after_shutdown: 0,
            allocation_rejected: true,
        }
    );
}

#[test]
fn real_vm_keeps_global_file_across_execute_then_drop_releases_windows_handle() {
    let report = real::vm_drop_releases_global_file().unwrap();
    assert!(report.same_vm_global_open);
    assert_eq!(report.contents, b"owned");
    assert_eq!(
        report.windows_exclusive_before_blocked,
        cfg!(windows).then_some(true)
    );
    assert_eq!(
        report.windows_exclusive_after_opened,
        cfg!(windows).then_some(true)
    );
}

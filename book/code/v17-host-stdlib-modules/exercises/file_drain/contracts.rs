use crate::model::{self, Report, Shutdown, Snapshot};

pub fn stage<'a>(report: &'a Report, name: &str) -> &'a Snapshot {
    report
        .snapshots
        .iter()
        .find(|snapshot| snapshot.stage == name)
        .unwrap()
}

// ANCHOR: file_drain_contract
pub fn drain_before_heap_shutdown(shutdown: Shutdown) {
    let report = model::run(shutdown).unwrap();
    let closed = stage(&report, "fixture.explicit_close");
    assert_eq!((closed.entries, closed.open_files), (3, 2));
    assert_eq!(closed.drops, [1, 0, 0]); // None 槽保留，原 File 已释放。
    let boundary = stage(&report, "heap.shutdown.begin");
    assert_eq!(boundary.entries, 0, "shutdown 前必须先清 files owner");
    assert_eq!(boundary.open_files, 0);
    assert_eq!(boundary.drops, [1, 1, 1]);
    assert_eq!(boundary.heap_objects, 4); // table + 三个仍可达 userdata。
    assert_eq!(report.reachable_before, [true; 3]);
    assert_eq!(report.released_objects, 4);
    assert_eq!(report.access_rejected_after_shutdown, [true; 3]);
    assert_eq!(stage(&report, "heap.shutdown.end").heap_objects, 0);
    assert_eq!(stage(&report, "drop.body.exit").open_files, 0);
    assert_eq!(report.after_owner_drops, [1, 1, 1]); // None 没有二次释放。
}
// ANCHOR_END: file_drain_contract

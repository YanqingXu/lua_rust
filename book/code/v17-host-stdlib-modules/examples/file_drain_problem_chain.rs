#[path = "../exercises/file_drain/faults.rs"]
mod faults;
#[path = "../exercises/file_drain/model.rs"]
pub mod model;
#[path = "../exercises/file_drain/real.rs"]
mod real;
#[path = "../exercises/file_drain/solution.rs"]
mod solution;
#[path = "../exercises/file_drain/support.rs"]
pub mod support;

fn trace(label: &str, shutdown: model::Shutdown) -> Result<(), String> {
    let report = model::run(shutdown)?;
    println!("{label}");
    for snapshot in report.snapshots {
        println!(
            "  {} entries={} open={} drops={:?} heap={}",
            snapshot.stage,
            snapshot.entries,
            snapshot.open_files,
            snapshot.drops,
            snapshot.heap_objects
        );
    }
    println!(
        "  after complete Rust drop: drops={:?}; reachable_before={:?}; access_rejected_after={:?}",
        report.after_owner_drops, report.reachable_before, report.access_rejected_after_shutdown
    );
    Ok(())
}

fn real_trace() -> Result<(), String> {
    let heap = real::heap_shutdown_discards_pending_events()?;
    println!("real Heap: {heap:?}");
    let vm = real::vm_drop_releases_global_file()?;
    println!(
        "real VM: global_open={} exclusive_before_blocked={:?} exclusive_after_opened={:?} contents={}",
        vm.same_vm_global_open,
        vm.windows_exclusive_before_blocked,
        vm.windows_exclusive_after_opened,
        String::from_utf8_lossy(&vm.contents)
    );
    if !cfg!(windows) {
        println!(
            "  OS release evidence unavailable: ordinary reopen is not exclusive on this platform"
        );
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    match mode.as_str() {
        "before" => trace("before: heap-only Drop body", faults::heap_only),
        "after" => trace("after: explicit files drain", solution::shutdown),
        "real" => real_trace(),
        "all" => {
            trace("before: heap-only Drop body", faults::heap_only)?;
            trace("after: explicit files drain", solution::shutdown)?;
            real_trace()
        }
        _ => Err("usage: file_drain_problem_chain [before|after|real|all]".into()),
    }
}

#![forbid(unsafe_code)]

use lua_core::gc::collector::{GarbageCollector, IncrementalPhase};
use lua_core::gc::gc_object::GcObject;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::value::Value;

#[test]
fn callback_unwind_preserves_list_accounting_and_incremental_child_edges() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    let child = gc.create(Table::new());
    let garbage = gc.create(Table::new());
    let owner = gc.create_root(Table::new());
    gc.begin_incremental_mark();
    gc.propagate_marks();
    assert!(
        gc.with_ref(owner, |table| table.gc_header().is_black())
            .unwrap()
    );
    assert!(
        gc.with_ref(child, |table| table.gc_header().is_white())
            .unwrap()
    );

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gc.with_mut(owner, |table| {
            for index in 1..=64 {
                table.set_array(index, &Value::Table(child));
            }
            panic!("injected failure after publishing a new child");
        })
        .unwrap();
    }));
    assert!(unwind.is_err());
    let actual_size: usize = [owner, child, garbage]
        .into_iter()
        .map(|table| gc.with_ref(table, GcObject::get_size).unwrap())
        .sum();
    assert_eq!(
        gc.total_memory(),
        actual_size,
        "unwind reconciles grown payloads"
    );
    gc.propagate_marks();
    gc.enter_incremental_atomic();
    gc.begin_incremental_sweep();
    while !gc.incremental_sweep_step(&mut strings, 1).2 {}
    assert_eq!(gc.complete_incremental_cycle(), 1);
    assert!(gc.contains_registered(owner));
    assert!(
        gc.contains_registered(child),
        "unwind executes the write barrier"
    );
    assert!(!gc.contains_registered(garbage));

    gc.with_mut(owner, |table| table.clear()).unwrap();
    gc.remove_root(owner);
    assert_eq!(
        gc.collect(&mut strings),
        2,
        "both list nodes remain reachable for teardown"
    );
    assert_eq!(gc.object_count(), 0);
    assert_eq!(gc.total_memory(), 0);
}

#[test]
fn publication_unwind_releases_protection_without_losing_managed_nodes() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    gc.create(Table::new());
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gc.with_publication(|transaction| {
            let table = transaction.alloc(Table::new());
            transaction
                .with_mut(&table, |table| {
                    table.set_array(1, &Value::Number(42.0));
                    panic!("injected publication callback failure");
                })
                .unwrap();
        });
    }));
    assert!(unwind.is_err());
    assert_eq!(gc.total_protected_root_count(), 0);
    assert_eq!(gc.object_count(), 2);
    assert_eq!(gc.collect(&mut strings), 2);
    assert_eq!(gc.total_memory(), 0);
}

#[test]
fn fixing_a_validated_object_preserves_children_and_rejects_foreign_handles() {
    let mut gc = GarbageCollector::new();
    let mut foreign = GarbageCollector::new();
    let mut strings = StringPool::new();
    let child = gc.create(Table::new());
    let owner = gc.create(Table::new());
    gc.with_mut(owner, |table| table.set_array(1, &Value::Table(child)))
        .unwrap();
    assert!(foreign.fix(owner).is_err());
    gc.begin_incremental_mark();
    gc.fix(owner).unwrap();
    gc.propagate_marks();
    gc.enter_incremental_atomic();
    gc.begin_incremental_sweep();
    while !gc.incremental_sweep_step(&mut strings, 1).2 {}
    assert_eq!(gc.complete_incremental_cycle(), 0);
    assert_eq!(gc.incremental_phase(), IncrementalPhase::Pause);
    assert_eq!(gc.collect(&mut strings), 0);
    assert!(gc.contains_registered(child));
}

#[test]
fn synchronous_sweep_retires_incremental_cursors_before_freeing_nodes() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    let tail = gc.create_root(Table::new());
    let head = gc.create_root(Table::new());
    gc.begin_incremental_mark();
    gc.propagate_marks();
    gc.enter_incremental_atomic();
    gc.begin_incremental_sweep();
    assert_eq!(gc.incremental_sweep_step(&mut strings, 1), (1, 0, false));
    assert_eq!(gc.sweep(&mut strings), 1);
    assert!(!gc.contains_registered(head));
    assert!(gc.contains_registered(tail));
    assert_eq!(gc.incremental_phase(), IncrementalPhase::Pause);
    let invalid_resume = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gc.incremental_sweep_step(&mut strings, 1)
    }));
    assert!(
        invalid_resume.is_err(),
        "invalid stage fails before touching old cursors"
    );
    gc.remove_root(tail);
    assert_eq!(gc.collect(&mut strings), 1);
    assert_eq!(gc.total_memory(), 0);
}

#[test]
fn synchronous_mark_abandons_incremental_progress_and_allows_a_fresh_cycle() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    let root = gc.create_root(Table::new());
    gc.begin_incremental_mark();
    gc.propagate_marks();
    gc.enter_incremental_atomic();
    gc.begin_incremental_sweep();
    gc.mark();
    assert_eq!(gc.incremental_phase(), IncrementalPhase::Pause);
    assert_eq!(gc.sweep(&mut strings), 0);
    gc.remove_root(root);
    assert_eq!(gc.collect(&mut strings), 1);
}

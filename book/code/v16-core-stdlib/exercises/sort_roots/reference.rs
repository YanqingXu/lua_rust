mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::{HeapError, Value};
use model::{key, Scenario, SortError, SortVm};

#[test]
fn snapshot_roots_survive_callbacks_and_restore_on_error() {
    contracts::snapshot_lifetime_and_cleanup(solution::with_snapshot);
}

#[test]
fn faulty_pair_roots_leave_later_snapshot_items_stale() {
    let mut vm = SortVm::new(Scenario::ClearAndCollect);
    assert!(matches!(
        vm.run_sort(faults::current_pair_only),
        Err(SortError::Heap(HeapError::StaleHandle { .. }))
    ));
    assert_eq!(vm.comparisons, 2);
    assert_eq!(
        vm.heap.table_get(vm.items[0], &key("rank")),
        Ok(Value::Number(4.0))
    );
    assert_eq!(
        vm.heap.table_get(vm.items[1], &key("rank")),
        Ok(Value::Number(3.0))
    );
    for victim in &vm.items[2..] {
        assert!(vm.heap.table_get(*victim, &key("rank")).is_err());
        assert_eq!(vm.heap.is_live(*victim), Ok(false));
    }
    assert_eq!(vm.temporary_roots, [Value::Table(vm.prefix)]);
}

#[test]
fn faulty_question_mark_leaks_snapshot_roots_after_callback_error() {
    let mut vm = SortVm::new(Scenario::ErrorAfterMutation);
    assert_eq!(
        vm.run_sort(faults::early_question_mark),
        Err(SortError::Comparator)
    );
    assert_eq!(vm.temporary_roots.len(), 5);
    assert_eq!(
        vm.heap.table_get(vm.source, &Value::Number(1.0)),
        Ok(Value::Number(99.0))
    );
    vm.collect().unwrap();
    assert_eq!(
        vm.heap.table_get(vm.items[0], &key("rank")),
        Ok(Value::Number(4.0))
    );
}

#[test]
fn real_vm_sort_with_unchanged_source_and_lua_truthiness() {
    assert_eq!(real::vm_normal_sort_and_truthiness(), Value::Boolean(true));
}

#[test]
fn real_vm_comparator_error_does_not_rollback_callback_mutation() {
    assert_eq!(
        real::vm_comparator_error_keeps_its_source_mutation(),
        Value::Boolean(true)
    );
}

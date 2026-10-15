mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::Value;
use model::{key, CycleVm, LoadError};

#[test]
fn loading_sentinel_is_visible_before_recursive_loader_call() {
    contracts::sentinel_precedes_recursive_loader(solution::run_loader);
}

#[test]
fn faulty_late_publication_exhausts_bounded_recursion() {
    for entry in ["self", "A"] {
        let mut vm = CycleVm::new(faults::publish_too_late, 6);
        assert_eq!(vm.require(entry), Err(LoadError::Budget));
        assert_eq!(vm.calls.values().sum::<usize>(), 6);
        assert_eq!(vm.heap.table_get(vm.loaded, &key(entry)), Ok(Value::Nil));
    }
}

#[test]
fn model_truthy_publication_does_not_mean_initialization_completed() {
    let mut vm = CycleVm::new(solution::run_loader, 6);
    let partial = vm.require("partial").unwrap();
    assert_eq!(vm.field(&partial, "saw_unready"), Ok(Value::Boolean(true)));
    assert_eq!(vm.field(&partial, "ready"), Ok(Value::Boolean(true)));
    assert_eq!(vm.field(&partial, "same"), Ok(Value::Boolean(true)));
    assert_eq!(vm.require("partial"), Ok(partial));
    assert_eq!(vm.require("done_true"), Ok(Value::Boolean(true)));
    assert_eq!(vm.require("done_true"), Ok(Value::Boolean(true)));
    assert_eq!(vm.calls.get("partial"), Some(&1));
    assert_eq!(vm.calls.get("done_true"), Some(&1));
}

#[test]
fn real_vm_preload_self_and_mutual_cycles_see_true_then_tables() {
    assert_eq!(real::vm_preload_cycles(), Ok(Value::Boolean(true)));
}

#[test]
fn real_vm_explicit_partial_exports_and_completed_true_use_the_same_cache() {
    assert_eq!(
        real::vm_explicit_partial_and_completed_true(),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn real_vm_file_modules_commit_final_exports_after_cycle_sentinel() {
    assert_eq!(
        real::vm_file_cycles(),
        Ok([Value::Boolean(true), Value::Boolean(true)])
    );
}

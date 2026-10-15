mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::Value;
use model::GlobalVm;

#[test]
fn single_environment_is_shared_by_globals_and_g() {
    contracts::single_environment(solution::install);
}

#[test]
fn faulty_snapshot_diverges_in_both_write_directions() {
    let mut vm = GlobalVm::new(faults::install_snapshot);
    assert_eq!(vm.global("seed"), vm.through_g("seed"));
    vm.set_global("forward", Value::Number(12.0)).unwrap();
    assert_eq!(vm.global("forward"), Ok(Value::Number(12.0)));
    assert_eq!(vm.through_g("forward"), Ok(Value::Nil));
    vm.set_through_g("reverse", Value::Number(34.0)).unwrap();
    assert_eq!(vm.through_g("reverse"), Ok(Value::Number(34.0)));
    assert_eq!(vm.global("reverse"), Ok(Value::Nil));
}

#[test]
fn faulty_snapshot_deletion_does_not_remove_the_other_copy() {
    let mut first = GlobalVm::new(faults::install_snapshot);
    first.set_global("seed", Value::Nil).unwrap();
    assert_eq!(first.global("seed"), Ok(Value::Nil));
    assert_eq!(first.through_g("seed"), Ok(Value::Number(1.0)));
    let mut second = GlobalVm::new(faults::install_snapshot);
    second.set_through_g("seed", Value::Nil).unwrap();
    assert_eq!(second.through_g("seed"), Ok(Value::Nil));
    assert_eq!(second.global("seed"), Ok(Value::Number(1.0)));
}

#[test]
fn self_referential_g_is_not_a_root_and_copied_handles_do_not_pin_it() {
    assert_eq!(
        real::heap_self_referential_environment_needs_a_root(),
        [true; 3]
    );
}

#[test]
fn real_vm_global_and_g_writes_and_nil_deletions_share_storage() {
    assert_eq!(real::vm_bidirectional(), Value::Boolean(true));
}

#[test]
fn real_vm_environment_keeps_reachable_graph_across_execute_and_gc() {
    assert_eq!(
        real::vm_reachable_graph_across_execute_and_gc(),
        std::array::from_fn(|_| Value::Boolean(true))
    );
}

#[test]
fn real_vm_rebinding_g_does_not_replace_the_environment() {
    assert_eq!(
        real::vm_rebinding_g_preserves_environment(),
        std::array::from_fn(|_| Value::Boolean(true))
    );
}

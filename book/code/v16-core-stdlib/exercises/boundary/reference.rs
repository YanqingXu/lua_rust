mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use model::{BoundaryError, BoundaryVm};

#[test]
fn protected_boundary_restores_depth_on_every_result() {
    contracts::restores_depth_before_returning(solution::invoke_boundary);
}

#[test]
fn faulty_question_mark_leaks_depth_and_rejects_a_later_legal_yield() {
    let mut vm = BoundaryVm::new(faults::early_question_mark, 0);
    assert_eq!(
        vm.invoke(model::ordinary_error),
        Err(BoundaryError::Ordinary("callback failed"))
    );
    assert_eq!(vm.depth, 1);
    assert_eq!(vm.try_yield(7), Err(BoundaryError::YieldDenied));
    assert!(vm.pending_values.is_none() && vm.pending_frames.is_empty());
}

#[test]
fn faulty_boolean_depth_opens_the_outer_boundary_and_publishes_pending_state() {
    let mut vm = BoundaryVm::new(faults::depth_as_boolean, 0);
    assert_eq!(
        vm.invoke(model::nested_after_caught_error),
        Err(BoundaryError::YieldSignal)
    );
    assert_eq!(vm.depth, 0);
    assert_eq!(vm.pending_values, Some(vec![99]));
    assert_eq!(vm.pending_frames, ["yield-frame"]);
    assert!(vm
        .trace
        .iter()
        .any(|line| line == "nested: after inner depth=0"));
}

fn expected() -> Vec<lua_core::Value> {
    use lua_core::Value::{Boolean, Nil, Number};
    vec![
        Boolean(true),
        Boolean(true),
        Number(41.0),
        Nil,
        Boolean(true),
        Boolean(true),
        Number(99.0),
        Boolean(true),
    ]
}

#[test]
fn real_vm_pcall_error_and_denied_yield_do_not_block_later_yield_in_same_execute() {
    assert_eq!(real::pcall_then_yield(), expected());
}

#[test]
fn real_vm_xpcall_target_and_handler_boundaries_restore_before_later_yield() {
    assert_eq!(real::xpcall_then_yield(), expected());
}

#[test]
fn real_vm_sort_failure_and_denied_comparator_yield_restore_before_later_yield() {
    assert_eq!(real::sort_then_yield(), expected());
}

#[test]
fn real_vm_callable_metamethod_boundary_restores_before_later_yield() {
    assert_eq!(real::callable_metamethod_then_yield(), expected());
}

#[test]
fn real_vm_nested_pcall_keeps_outer_boundary_until_it_returns() {
    assert_eq!(real::nested_pcall_then_yield(), expected());
}

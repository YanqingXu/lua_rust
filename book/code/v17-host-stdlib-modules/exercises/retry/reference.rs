mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::Value;

#[test]
fn failure_removes_only_the_current_loading_entry() {
    contracts::failed_entry_is_retryable(solution::attempt);
}

#[test]
fn faulty_question_mark_turns_retry_into_a_cached_true() {
    let mut vm = model::RetryVm::new(faults::question_mark);
    assert_eq!(
        vm.require("broken", model::fail),
        Err(model::loader_error("broken"))
    );
    assert_eq!(vm.cached("broken"), Value::Boolean(true));
    assert_eq!(
        vm.require("broken", model::repaired),
        Ok(Value::Boolean(true))
    );
    assert_eq!(vm.calls["broken"], 1);
}

#[test]
fn faulty_clear_all_discards_successful_dependency_and_outer_cache() {
    let mut vm = model::RetryVm::new(faults::clear_all);
    assert!(vm.require("parent", model::depends_then_fails).is_err());
    assert_eq!(vm.cached("dependency"), Value::Nil);
    assert_eq!(vm.cached("outer"), Value::Nil);
    assert_eq!(
        vm.require("parent", model::depends_then_succeeds),
        Ok(Value::Number(42.0))
    );
    assert_eq!(vm.calls["dependency"], 2);
}

#[test]
fn real_vm_preload_failure_published_value_is_removed_before_same_execute_retry() {
    assert_eq!(real::preload_retry(), Value::Boolean(true));
}

#[test]
fn real_vm_parent_failure_keeps_a_successful_dependency_cached() {
    assert_eq!(
        real::nested_dependency_survives_parent_failure(),
        Value::Boolean(true)
    );
}

#[test]
fn real_vm_require_rejects_yield_then_restores_a_later_legal_yield() {
    assert_eq!(
        real::rejected_yield_then_retry_and_yield(),
        Value::Boolean(true)
    );
}

#[test]
fn real_vm_missing_file_can_be_created_and_retried_without_replacing_vm() {
    assert_eq!(
        real::missing_file_can_be_created_and_retried(),
        std::array::from_fn(|_| Value::Boolean(true))
    );
}

#[test]
fn real_vm_compile_error_stops_before_later_template_then_retries() {
    assert_eq!(
        real::compile_error_stops_before_a_later_template_then_retries(),
        std::array::from_fn(|_| Value::Boolean(true))
    );
}

#[test]
fn real_vm_runtime_failure_clears_marker_then_rewritten_file_retries() {
    assert_eq!(
        real::runtime_failure_clears_marker_then_same_vm_retries(),
        std::array::from_fn(|_| Value::Boolean(true))
    );
}

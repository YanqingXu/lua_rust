mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::Value::{Boolean, Nil, Number};

#[test]
fn pcall_keeps_the_complete_result_window() {
    contracts::complete_pcall_results(solution::pack_pcall);
}

#[test]
fn xpcall_only_handles_failures_and_selects_one_result() {
    contracts::conditional_xpcall_handler(solution::pack_xpcall);
}

#[test]
fn faulty_pcall_collapses_the_result_window() {
    assert_eq!(
        faults::first_result_only(Ok(vec![Number(7.0), Nil, Number(9.0), Nil])),
        [Boolean(true), Number(7.0)]
    );
    assert_eq!(faults::first_result_only(Ok(vec![])), [Boolean(true), Nil]);
}

#[test]
fn faulty_xpcall_runs_handler_after_success_and_leaks_its_extra_results() {
    let mut calls = 0;
    let actual = faults::always_handle_and_forward_all(Ok(vec![Number(7.0)]), &mut |_| {
        calls += 1;
        Ok(vec![Number(42.0), Number(99.0)])
    });
    assert_eq!(calls, 1);
    assert_eq!(actual, [Boolean(false), Number(42.0), Number(99.0)]);
}

#[test]
fn real_vm_pcall_preserves_result_count_and_trailing_nil() {
    assert_eq!(
        real::pcall_multi_values(),
        [
            Number(5.0),
            Boolean(true),
            Number(7.0),
            Nil,
            Number(9.0),
            Nil
        ]
    );
}

#[test]
fn real_vm_continues_after_error_in_the_same_execute_and_releases_argument() {
    assert_eq!(
        real::run(real::ERROR_THEN_CONTINUE),
        [
            Boolean(true),
            Boolean(true),
            Number(42.0),
            Number(41.0),
            Boolean(true)
        ]
    );
}

#[test]
fn real_vm_xpcall_success_does_not_invoke_the_handler() {
    assert_eq!(
        real::xpcall_success_skips_handler(),
        [
            Boolean(true),
            Number(7.0),
            Nil,
            Number(9.0),
            Nil,
            Number(0.0)
        ]
    );
}

#[test]
fn real_vm_xpcall_handles_first_empty_nil_and_handler_errors() {
    assert_eq!(
        real::xpcall_handler_variants(),
        [
            Boolean(true),
            Number(42.0),
            Boolean(true),
            Number(1.0),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Number(2.0),
            Boolean(true),
            Number(42.0)
        ]
    );
}

#[test]
fn real_vm_errors_are_strings_and_noncallable_body_is_catchable() {
    assert_eq!(
        real::errors_are_formatted_strings(),
        [
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Boolean(true),
            Number(42.0),
            Boolean(true),
            Boolean(true),
            Number(42.0)
        ]
    );
}

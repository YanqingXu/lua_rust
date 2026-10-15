#[path = "../support/bytecode_process.rs"]
pub mod bytecode_process;
mod contracts;
mod faults;
pub mod model;
#[path = "../support/process.rs"]
pub mod process;
pub mod real;
mod solution;

use lua_core::{Opcode, Value};
use lua_vm::VmErrorKind;
use std::path::Path;

#[test]
fn records_current_registers_before_dispatch() {
    contracts::records_current_registers_before_dispatch(solution::dispatch);
}

#[test]
fn faulty_after_recording_shows_current_destination_too_early() {
    let events = model::run(faults::record_after).unwrap();
    assert_eq!(events[0].registers, [Value::Number(40.0), Value::Nil]);
    assert_eq!(
        events[2].registers,
        [Value::Number(42.0), Value::Number(2.0)]
    );
}

#[test]
fn faulty_slot_aliases_rewrite_every_historical_state() {
    assert_eq!(
        faults::alias_history().unwrap(),
        vec![vec![Value::Number(42.0), Value::Number(2.0)]; 4]
    );
}

#[test]
fn real_vm_trace_matches_before_values_metadata_and_json_step() {
    let (result, events) = real::arithmetic_events().unwrap();
    assert_eq!(result, [Value::Number(42.0)]);
    contracts::assert_arithmetic_before(&events);
    let json = lua_bytecode::trace_json(&events);
    let prefixes = [
        "\"step\":0,\"depth\":0,\"function\":\"trace-before\",\"pc\":0,\"line\":10,\"op\":\"LoadK\"",
        "\"step\":1,\"depth\":0,\"function\":\"trace-before\",\"pc\":1,\"line\":11,\"op\":\"LoadK\"",
        "\"step\":2,\"depth\":0,\"function\":\"trace-before\",\"pc\":2,\"line\":12,\"op\":\"Add\"",
        "\"step\":3,\"depth\":0,\"function\":\"trace-before\",\"pc\":3,\"line\":13,\"op\":\"Return\"",
    ];
    assert!(prefixes.iter().all(|prefix| json.contains(prefix)));
}

#[test]
fn real_branch_skips_pc_while_json_steps_remain_contiguous() {
    let (result, events) = real::branch_events().unwrap();
    assert_eq!(result, [Value::Number(42.0)]);
    assert_eq!(
        events.iter().map(|event| event.pc).collect::<Vec<_>>(),
        [0, 1, 3, 4]
    );
    assert_eq!(events[2].opcode, Opcode::LoadK);
    assert_eq!(events[2].registers, [Value::Boolean(false), Value::Nil]);
    assert_eq!(
        events[3].registers,
        [Value::Boolean(false), Value::Number(42.0)]
    );
    assert!(lua_bytecode::trace_json(&events)
        .contains("\"step\":2,\"depth\":0,\"function\":\"trace-branch\",\"pc\":3,\"line\":23"));
}

#[test]
fn real_trace_is_default_off_take_disables_and_enable_clears_history() {
    assert_eq!(real::control_counts().unwrap(), [0, 8, 0, 4, 0]);
}

#[test]
fn failed_instruction_is_recorded_but_decode_and_budget_fail_before_recording() {
    let report = real::failure_boundaries().unwrap();
    assert_eq!(report.failed_load.pc, 0);
    assert_eq!(report.failed_load.line, 10);
    assert!(report.failed_load.message.contains("constant index"));
    assert_eq!(report.load_events.len(), 1);
    assert_eq!(report.load_events[0].opcode, Opcode::LoadK);
    assert_eq!(report.load_events[0].registers, [Value::Nil, Value::Nil]);
    assert!(report.invalid.message.contains("invalid opcode 63"));
    assert!(report.invalid_events.is_empty());
    assert_eq!(report.budget.kind, VmErrorKind::Limit);
    assert_eq!(report.budget.pc, 2);
    assert_eq!(
        report
            .budget_events
            .iter()
            .map(|event| event.pc)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(report.zero_budget_events, 0);
}

#[test]
fn real_trace_keeps_typed_identity_but_neither_deep_state_nor_gc_root() {
    assert_eq!(
        real::identity_is_not_deep_snapshot_or_gc_root().unwrap(),
        real::IdentityReport {
            earlier_handle_found: true,
            current_field: Value::Number(42.0),
            internal_trace_contains_reclaimed_handle: true,
            republish_rejected_as_stale: true,
        }
    );
}

#[test]
fn real_trace_binary_success_error_and_lua_print_obey_stream_contracts() {
    let binary =
        bytecode_process::sibling_of_lua_app(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
    let report = real::binary_cases(&binary).unwrap();
    assert_eq!(report.pure_events, 5);
    assert_eq!(report.runtime_exit, 1);
    assert_eq!(report.runtime_stdout_bytes, 0);
    assert!(report.partial_library_events > 0);
    assert!(report.print_pollutes_stdout);
}

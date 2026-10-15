mod contracts;
mod faults;
mod model;
#[path = "../support/process.rs"]
mod process;
mod real;
mod solution;
#[path = "../support/temp.rs"]
mod temp;

use lua_core::Value;
use model::{CliError, Outcome};
use std::path::Path;

#[test]
fn result_boundary_preserves_streams_and_maps_exit_codes() {
    contracts::result_boundary(solution::finish);
}

#[test]
fn printing_an_error_without_failure_status_reports_success() {
    let result = faults::print_error_but_succeed(Outcome {
        stdout: vec![],
        result: Err(CliError::Runtime("broken".to_owned())),
    });
    assert_eq!(result.code, 0);
    assert_eq!(result.stderr, b"lua: broken\n");
}

#[test]
fn one_error_code_loses_usage_classification() {
    let result = faults::all_errors_one(Outcome {
        stdout: vec![],
        result: Err(CliError::Usage("bad option".to_owned())),
    });
    assert_eq!(result.code, 1);
    assert!(String::from_utf8(result.stderr).unwrap().contains("usage:"));
}

#[test]
fn printing_script_returns_adds_unrequested_stdout() {
    let result = faults::print_script_results(Outcome {
        stdout: b"kept\n".to_vec(),
        result: Ok(vec![Value::Number(42.0)]),
    });
    assert_eq!(result.code, 0);
    assert_eq!(result.stdout, b"kept\n42\n");
}

fn verify(probes: Vec<real::Probe>, count: usize) {
    assert_eq!(probes.len(), count);
    for probe in probes {
        assert!(!probe.name.is_empty());
        assert!(matches!(probe.code, 0..=2));
        assert!(
            probe.stdout.len() < 1024
                && matches!(probe.stderr_class, "empty" | "usage" | "failure")
        );
    }
}

#[test]
fn real_process_help_version_unknown_and_missing_operand_streams() {
    verify(
        real::help_version_and_usage(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap(),
        7,
    );
}

#[test]
fn real_process_compile_runtime_file_failures_and_silent_script_returns() {
    verify(
        real::compile_runtime_file_and_silent_returns(Path::new(env!("CARGO_BIN_EXE_lua_app")))
            .unwrap(),
        6,
    );
}

#[test]
fn real_process_failed_execute_or_load_stops_later_actions_script_and_repl() {
    verify(
        real::failed_action_skips_later_execute_script_and_repl(Path::new(env!(
            "CARGO_BIN_EXE_lua_app"
        )))
        .unwrap(),
        2,
    );
}

#[test]
fn real_process_failed_script_or_missing_file_stops_repl() {
    verify(
        real::failed_script_or_file_skips_repl(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap(),
        2,
    );
}

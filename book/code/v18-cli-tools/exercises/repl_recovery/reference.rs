mod contracts;
mod faults;
pub mod model;
#[path = "../support/process.rs"]
pub mod process;
pub mod real;
mod solution;

use lua_app::AppError;
use lua_core::Value;
use model::{Event, Repl};
use std::path::Path;

#[test]
fn source_lifecycle_recovers_after_errors() {
    contracts::source_lifecycle_recovers_after_errors(solution::submit);
}

#[test]
fn faulty_clear_every_line_discards_block_and_runs_false_branch() {
    let mut repl = Repl::new(faults::clear_every_line);
    contracts::values(repl.push("saved=40"), &[]);
    assert!(matches!(repl.push("if false then"), Event::Incomplete(_)));
    assert!(repl.source.is_empty());
    contracts::values(repl.push("saved=99"), &[]);
    assert!(matches!(
        repl.push("end"),
        Event::Error(AppError::Compile(_))
    ));
    contracts::values(repl.push("=saved"), &[Value::Number(99.0)]);
}

#[test]
fn faulty_retained_errors_pollute_next_input_and_repeat_runtime_effects() {
    let mut compile = Repl::new(faults::keep_after_error);
    assert!(matches!(
        compile.push("local ="),
        Event::Error(AppError::Compile(_))
    ));
    assert!(matches!(
        compile.push("=42"),
        Event::Error(AppError::Compile(_))
    ));
    assert_eq!(compile.source, "local =\n=42\n");
    let mut runtime = Repl::new(faults::keep_after_error);
    contracts::values(runtime.push("effects=0"), &[]);
    assert!(matches!(
        runtime.push("effects=effects+1;error('recover')"),
        Event::Error(AppError::Runtime(_))
    ));
    assert!(matches!(
        runtime.push("return effects"),
        Event::Error(AppError::Runtime(_))
    ));
    assert_eq!(
        runtime.inspect(b"return effects").unwrap(),
        [Value::Number(2.0)]
    );
}

#[test]
fn faulty_question_mark_aborts_before_the_next_input() {
    let mut repl = Repl::new(solution::submit);
    assert!(
        faults::stop_at_first_error(&mut repl, &["saved=40", "error('recover')", "saved=42"])
            .is_err()
    );
    assert_eq!(repl.submitted, 2);
    assert!(repl.source.is_empty());
    assert_eq!(
        repl.inspect(b"return saved").unwrap(),
        [Value::Number(40.0)]
    );
}

#[test]
fn heuristic_is_not_a_complete_continuation_parser() {
    let error = lua_compiler::compile(b"return 40 +\n").unwrap_err();
    assert_eq!(error.message, "expected an expression");
    assert!(!model::is_incomplete(&error));
    assert!(lua_compiler::compile(b"return 40 +\n2\n").is_ok());
    let mut repl = Repl::new(solution::submit);
    assert!(matches!(
        repl.push("return 40 +"),
        Event::Error(AppError::Compile(_))
    ));
    assert!(repl.source.is_empty());
    assert!(matches!(repl.push("2"), Event::Error(AppError::Compile(_))));
    contracts::values(repl.push("=42"), &[Value::Number(42.0)]);
}

#[test]
fn real_cli_interactive_recovery_preserves_globals_effects_and_result_arity() {
    assert_eq!(
        real::interactive_recovery(Path::new(env!("CARGO_BIN_EXE_lua_app")))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn real_cli_exit_codes_pending_eof_and_whole_stdin_chunk_match_snapshot() {
    assert_eq!(
        real::mode_boundaries(Path::new(env!("CARGO_BIN_EXE_lua_app")))
            .unwrap()
            .len(),
        6
    );
}

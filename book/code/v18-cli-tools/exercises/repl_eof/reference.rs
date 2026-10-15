mod contracts;
pub mod faults;
pub mod model;
#[path = "../support/process.rs"]
pub mod process;
pub mod real;
mod solution;

use lua_app::{AppError, Repl, ReplEvent, Session};
use lua_compiler::CompileErrorKind;
use lua_core::Value;
use model::PendingInput;
use std::path::Path;

#[test]
fn eof_reports_pending_once_and_releases_buffer() {
    contracts::eof_reports_pending_once_and_releases_buffer(solution::finish);
}

#[test]
fn faulty_discard_pending_reports_success_and_loses_original_diagnostic() {
    let mut pending = PendingInput::incomplete("return 40 +\n");
    assert!(faults::discard_pending(&mut pending).is_ok());
    assert!(pending.source.is_empty());
    assert!(pending.diagnostic.is_none());
}

#[test]
fn faulty_clone_without_consuming_repeats_diagnostic_and_keeps_source() {
    let mut pending = PendingInput::incomplete("return 40 +\n");
    let first = faults::clone_without_consuming(&mut pending).unwrap_err();
    let second = faults::clone_without_consuming(&mut pending).unwrap_err();
    assert_eq!(first, second);
    assert!(!pending.source.is_empty());
    assert!(pending.diagnostic.is_some());
}

#[test]
fn actual_repl_eof_does_not_execute_prefix_and_consumes_diagnostic_once() {
    let mut session = Session::new();
    session.execute_multi(b"effects=0").unwrap();
    {
        let mut repl = Repl::new(&mut session);
        assert!(matches!(
            repl.submit("effects=effects+1; if true then"),
            Ok(ReplEvent::NeedMoreInput)
        ));
        assert!(matches!(repl.finish_eof(), Err(AppError::Compile(error))
            if error.kind == CompileErrorKind::NeedMoreInput));
        assert!(repl.source().is_empty());
        assert!(!repl.is_pending());
        assert!(repl.finish_eof().is_ok());
    }
    assert_eq!(
        session.execute_multi(b"return effects").unwrap(),
        [Value::Number(0.0)]
    );
}

#[test]
fn real_cli_distinguishes_idle_pending_invalid_and_final_line_eof() {
    let cases = real::eof_cases(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
    assert_eq!(cases.len(), 10);
    assert_eq!(
        cases
            .iter()
            .find(|case| case.name == "short string final fragment has EOF instead of LF")
            .unwrap()
            .stderr,
        b"lua: 1:15: unterminated short string\n"
    );
    assert_eq!(
        cases
            .iter()
            .find(|case| case.name == "short string newline is invalid and recovers")
            .unwrap()
            .stderr,
        b"lua: 1:15: newline in short string\n"
    );
}

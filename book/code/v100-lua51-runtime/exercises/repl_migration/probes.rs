//! These assertions call only this snapshot's public compiler/controller API.
use crate::{faults, model::PendingInput};
use lua_app::{run_repl, AppError, Repl, ReplEvent, Session};
use lua_compiler::{compile, CompileErrorKind};
use lua_core::Value;
use std::io::Cursor;

pub fn historical_message_heuristic_misses_structured_kind() {
    let pending = compile(b"return 40 +\n").unwrap_err();
    assert_eq!(pending.kind, CompileErrorKind::NeedMoreInput);
    assert!(!faults::historical_message_heuristic(&pending));
    assert!(compile(b"return 40 +\n2\n").is_ok());
    let mut invalid = compile(b"return @\n").unwrap_err();
    assert_eq!(invalid.kind, CompileErrorKind::Invalid);
    // A wording-only injection is not presented as a real compiler diagnostic.
    invalid.message = "unterminated diagnostic wording is not a kind".into();
    assert!(faults::historical_message_heuristic(&invalid));
    assert_eq!(invalid.kind, CompileErrorKind::Invalid);
}

pub fn silent_eof_drops_original_diagnostic() {
    let mut input = PendingInput::incomplete("return 40 +\n");
    assert!(faults::silent_eof(&mut input).is_ok());
    assert!(input.source.is_empty());
    assert!(input.diagnostic.is_none());
}

pub fn cloned_eof_diagnostic_is_reported_twice() {
    let mut input = PendingInput::incomplete("return 40 +\n");
    let first = faults::clone_eof(&mut input).unwrap_err();
    let second = faults::clone_eof(&mut input).unwrap_err();
    assert_eq!(first, second);
    assert!(!input.source.is_empty());
    assert!(input.diagnostic.is_some());
}

// ANCHOR: repl_migration_real_controller
pub fn local_v100_controller_executes_once_and_recovers() {
    let mut session = Session::new();
    session.execute_multi(b"effects=0; saved=40").unwrap();
    {
        let mut repl = Repl::new(&mut session);
        assert!(matches!(
            repl.submit("effects=effects+1; local answer ="),
            Ok(ReplEvent::NeedMoreInput)
        ));
        assert!(matches!(repl.submit("40+2"), Ok(ReplEvent::Values(values))
            if values.is_empty()));
        assert!(
            matches!(repl.submit("local ="), Err(AppError::Compile(error))
            if error.kind == CompileErrorKind::Invalid)
        );
        assert!(repl.source().is_empty());
        assert!(
            matches!(repl.submit("=saved+2,effects"), Ok(ReplEvent::Values(values))
            if values == [Value::Number(42.0), Value::Number(1.0)])
        );
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
        session.execute_multi(b"return saved,effects").unwrap(),
        [Value::Number(40.0), Value::Number(1.0)]
    );
}
// ANCHOR_END: repl_migration_real_controller

// ANCHOR: repl_migration_real_transport
pub fn local_v100_driver_distinguishes_final_fragment_from_newline() {
    let mut session = Session::new();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let result = run_repl(
        &mut session,
        Cursor::new(b"return 'broken"),
        &mut output,
        &mut errors,
        false,
    );
    assert!(matches!(result, Err(AppError::Compile(error))
        if error.kind == CompileErrorKind::NeedMoreInput
            && error.to_string() == "1:15: unterminated short string"));
    assert!(output.is_empty());
    assert!(errors.is_empty()); // The caller prints the returned EOF error once.
    run_repl(
        &mut session,
        Cursor::new(b"return 'broken\n=42"),
        &mut output,
        &mut errors,
        false,
    )
    .unwrap();
    assert_eq!(output, b"42\n");
    assert_eq!(errors, b"lua: 1:15: newline in short string\n");
}
// ANCHOR_END: repl_migration_real_transport

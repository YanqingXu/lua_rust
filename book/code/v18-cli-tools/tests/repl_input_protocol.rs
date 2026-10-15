use lua_app::{run_repl, AppError, Repl, ReplEvent, Session};
use lua_compiler::CompileErrorKind;
use lua_core::Value;
use std::io::{self, BufRead, Cursor, Read};

#[test]
fn pending_chunks_execute_once_and_keep_the_same_session() {
    let mut session = Session::new();
    session.execute_multi(b"effects=0; saved=40").unwrap();
    {
        let mut repl = Repl::new(&mut session);
        assert!(matches!(
            repl.submit("effects=effects+1; local value ="),
            Ok(ReplEvent::NeedMoreInput)
        ));
        assert!(matches!(repl.submit("40 +"), Ok(ReplEvent::NeedMoreInput)));
        assert!(matches!(repl.submit("2"), Ok(ReplEvent::Values(values)) if values.is_empty()));
        assert!(repl.source().is_empty());
        assert!(!repl.is_pending());
        assert!(
            matches!(repl.submit("=saved+2,effects"), Ok(ReplEvent::Values(values))
            if values == [Value::Number(42.0), Value::Number(1.0)])
        );
        assert!(repl.finish_eof().is_ok());
    }
    assert_eq!(
        session.execute_multi(b"return effects,saved").unwrap(),
        [Value::Number(1.0), Value::Number(40.0)]
    );
}

#[test]
fn ordinary_errors_clear_source_and_continue_without_rollback() {
    let mut session = Session::new();
    let mut repl = Repl::new(&mut session);
    assert!(matches!(
        repl.submit("saved=40; effects=0"),
        Ok(ReplEvent::Values(_))
    ));
    assert!(
        matches!(repl.submit("local ="), Err(AppError::Compile(error))
        if error.kind == CompileErrorKind::Invalid)
    );
    assert!(repl.source().is_empty());
    assert!(matches!(
        repl.submit("effects=effects+1;error('recover')"),
        Err(AppError::Runtime(_))
    ));
    assert!(repl.source().is_empty());
    assert!(
        matches!(repl.submit("=effects,saved"), Ok(ReplEvent::Values(values))
        if values == [Value::Number(1.0), Value::Number(40.0)])
    );
    assert!(repl.finish_eof().is_ok());
}

#[test]
fn expression_shorthand_is_only_rewritten_while_idle() {
    let mut session = Session::new();
    let mut repl = Repl::new(&mut session);
    assert!(matches!(
        repl.submit("  =40 +"),
        Ok(ReplEvent::NeedMoreInput)
    ));
    assert_eq!(repl.source(), "return 40 +\n");
    assert!(matches!(repl.submit("2"), Ok(ReplEvent::Values(values))
        if values == [Value::Number(42.0)]));
    assert!(matches!(
        repl.submit("if true then"),
        Ok(ReplEvent::NeedMoreInput)
    ));
    assert!(matches!(repl.submit("=42"), Err(AppError::Compile(error))
        if error.kind == CompileErrorKind::Invalid));
    assert!(repl.source().is_empty());
    assert!(matches!(repl.submit("==42"), Err(AppError::Compile(_))));
    assert!(repl.source().is_empty());
}

#[test]
fn unescaped_short_string_newline_is_an_error_and_does_not_swallow_next_line() {
    let mut session = Session::new();
    let mut repl = Repl::new(&mut session);
    assert!(
        matches!(repl.submit("return 'broken"), Err(AppError::Compile(error))
        if error.kind == CompileErrorKind::Invalid)
    );
    assert!(!repl.is_pending());
    assert!(matches!(repl.submit("=42"), Ok(ReplEvent::Values(values))
        if values == [Value::Number(42.0)]));

    // A final fragment without LF has reached EOF, not an unescaped newline.
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
    assert!(errors.is_empty()); // The caller receives the one EOF diagnostic.
}

struct InputFailure(Cursor<Vec<u8>>);

impl Read for InputFailure {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for InputFailure {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let bytes = self.0.fill_buf()?;
        if bytes.is_empty() {
            Err(io::Error::other("injected input failure"))
        } else {
            Ok(bytes)
        }
    }

    fn consume(&mut self, count: usize) {
        self.0.consume(count);
    }
}

#[test]
fn input_error_is_not_converted_to_pending_eof() {
    let mut session = Session::new();
    session.execute_multi(b"effects=0").unwrap();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let result = run_repl(
        &mut session,
        InputFailure(Cursor::new(b"effects=effects+1; if true then\n".to_vec())),
        &mut output,
        &mut errors,
        false,
    );
    assert!(matches!(result, Err(AppError::Host(message))
        if message.contains("cannot read REPL input: injected input failure")));
    assert!(output.is_empty());
    assert!(errors.is_empty());
    assert_eq!(
        session.execute_multi(b"return effects").unwrap(),
        [Value::Number(0.0)]
    );
}

#[test]
fn repl_output_keeps_binary_values_and_distinguishes_nil_from_no_values() {
    let mut session = Session::new();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    run_repl(
        &mut session,
        Cursor::new(b"return nil\nreturn\nreturn string.char(0,255),42\n"),
        &mut output,
        &mut errors,
        false,
    )
    .unwrap();
    assert_eq!(output, b"nil\n\0\xff\t42\n");
    assert!(errors.is_empty());
}

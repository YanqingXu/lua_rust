mod contracts;
mod faults;
mod model;
#[path = "../support/process.rs"]
mod process;
mod real;
mod solution;

use lua_compiler::{compile, CompileErrorKind};
use model::Decision;

#[test]
fn structured_errors_choose_the_next_repl_action() {
    contracts::structured_errors_choose_the_next_repl_action(solution::decide);
}

#[test]
fn message_keywords_miss_an_expression_that_a_new_line_can_complete() {
    let compilation = compile(b"return 40 +\n");
    assert_eq!(faults::keywords(&compilation), Decision::Reject);
    assert_eq!(solution::decide(&compilation), Decision::Preserve);
    assert!(compile(b"return 40 +\n2\n").is_ok());
}

#[test]
fn an_error_at_eof_does_not_imply_that_a_new_line_can_repair_it() {
    for source in [b"return 1e".as_slice(), b"return 0x", b"return '\\256"] {
        let compilation = compile(source);
        assert_eq!(
            compilation.as_ref().unwrap_err().position.offset,
            source.len()
        );
        assert_eq!(
            faults::eof_position(&compilation, source.len()),
            Decision::Preserve
        );
        assert_eq!(solution::decide(&compilation), Decision::Reject);
        assert!(compile(&[source, b"\n2\n"].concat()).is_err());
    }
}

#[test]
fn diagnostic_wording_cannot_change_the_structured_decision() {
    let mut pending = compile(b"return 40 +\n").unwrap_err();
    pending.message = "please finish this expression".into();
    assert_eq!(solution::decide(&Err(pending)), Decision::Preserve);
    let mut syntax = compile(b"return @\n").unwrap_err();
    syntax.message = "unterminated diagnostic wording is not a kind".into();
    let compilation = Err(syntax);
    assert_eq!(solution::decide(&compilation), Decision::Reject);
    assert_eq!(faults::keywords(&compilation), Decision::Preserve);
}

#[test]
fn raw_newline_short_strings_and_unclosed_long_tokens_have_distinct_kinds() {
    for source in [b"return 'hello\n".as_slice(), b"return \"hello\n"] {
        assert_eq!(compile(source).unwrap_err().kind, CompileErrorKind::Invalid);
    }
    for source in [b"return [=[hello\n".as_slice(), b"--[=[hello\n"] {
        assert_eq!(
            compile(source).unwrap_err().kind,
            CompileErrorKind::NeedMoreInput
        );
    }
}

#[test]
fn completed_inputs_run_the_real_proto_without_changing_the_compile_api() {
    use lua_core::{ByteString, Value};
    for (name, prefix, suffix) in model::unfinished() {
        let values = model::complete(&[prefix, suffix].concat()).unwrap();
        let expected = if name == "long string" {
            Value::String(ByteString::new(b"hello\nworld".as_slice()))
        } else {
            Value::Number(42.0)
        };
        assert_eq!(values, [expected], "{name}");
    }
}

#[test]
fn real_binary_continues_only_unfinished_input_and_recovers_from_syntax() {
    let cases = real::cases(std::path::Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
    assert_eq!(cases.len(), 4);
    assert_eq!(cases[0].name, "expression resumes");
    assert!(cases.iter().all(|case| !case.stdout.is_empty()));
    assert_eq!(
        cases.iter().filter(|case| !case.stderr.is_empty()).count(),
        2
    );
}

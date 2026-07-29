//! Byte-oriented source and string literal regression tests.

use lua_compiler::ast::expr::Expr;
use lua_compiler::ast::stmt::Stmt;
use lua_compiler::codegen::CodeGenerator;
use lua_compiler::lexer::Lexer;
use lua_compiler::parser::Parser;
use lua_compiler::token::{Token, TokenValue};
use lua_core::gc::collector::GarbageCollector;
use lua_core::string_pool::StringPool;
use lua_core::value::Value;

fn string_bytes(token: &Token) -> &[u8] {
    match &token.value {
        TokenValue::String(value) => value.as_bytes(),
        other => panic!("expected string token, got {other:?}"),
    }
}

#[test]
fn byte_lexer_preserves_raw_high_bytes_and_embedded_nul_in_short_strings() {
    let source = [b'"', 0x00, 0x80, 0xff, b'"'];
    let mut lexer = Lexer::from_bytes(&source);

    let token = lexer.next_token();

    assert_eq!(string_bytes(&token), &[0x00, 0x80, 0xff]);
    assert_eq!(token.lexeme.as_bytes(), source);
}

#[test]
fn decimal_escapes_produce_exact_bytes_and_reject_values_above_255() {
    let mut valid = Lexer::from_bytes(br#""\000\128\255""#);
    assert_eq!(string_bytes(&valid.next_token()), &[0x00, 0x80, 0xff]);

    let mut invalid = Lexer::from_bytes(br#""\256""#);
    let error = invalid.next_token();
    assert!(error.is_error());
    assert_eq!(error.error_message, "decimal escape too large");
}

#[test]
fn lua51_unknown_escape_drops_only_the_backslash() {
    let mut lexer = Lexer::from_bytes(br#""\xFF""#);

    assert_eq!(string_bytes(&lexer.next_token()), b"xFF");
}

#[test]
fn long_strings_preserve_bytes_and_normalize_newlines() {
    let source = b"[=[\r\n\0\x80\xff\r\nx\n\ry]=]";
    let mut lexer = Lexer::from_bytes(source);

    assert_eq!(
        string_bytes(&lexer.next_token()),
        &[0x00, 0x80, 0xff, b'\n', b'x', b'\n', b'y']
    );
}

#[test]
fn mismatched_long_string_delimiters_remain_content() {
    let mut lexer = Lexer::from_bytes(b"[=[a]==]b]=]");

    assert_eq!(string_bytes(&lexer.next_token()), b"a]==]b");
}

#[test]
fn nul_outside_a_string_is_not_treated_as_eof() {
    let mut lexer = Lexer::from_bytes(b"a\0b");

    assert_eq!(lexer.next_token().lexeme.as_bytes(), b"a");
    let nul = lexer.next_token();
    assert!(nul.is_error());
    assert_eq!(nul.lexeme.as_bytes(), b"\0");
    assert_eq!(lexer.next_token().lexeme.as_bytes(), b"b");
}

#[test]
fn parser_carries_literal_bytes_into_the_ast() {
    let source = b"return \"\0\x80\xff\"";
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse().expect("byte source should parse");
    let Stmt::Return(return_stmt) = &*chunk.statements[0] else {
        panic!("expected return statement");
    };

    assert!(
        matches!(&*return_stmt.values[0], Expr::String(value) if value.value.as_bytes() == [0x00, 0x80, 0xff])
    );
}

#[test]
fn utf8_parser_wrapper_preserves_the_text_encoding_bytes() {
    let mut parser = Parser::new("return \"é\"");
    let chunk = parser.parse().expect("UTF-8 wrapper source should parse");
    let Stmt::Return(return_stmt) = &*chunk.statements[0] else {
        panic!("expected return statement");
    };

    assert!(
        matches!(&*return_stmt.values[0], Expr::String(value) if value.value.as_bytes() == "é".as_bytes())
    );
}

#[test]
fn invalid_syntax_bytes_are_escaped_in_diagnostics_without_replacement() {
    let mut parser = Parser::from_bytes(b"return \xff");
    let error = parser.parse().expect_err("invalid syntax byte must fail");

    assert!(error.message.contains("\\xff"), "{}", error.message);
    assert!(!error.message.contains('\u{fffd}'), "{}", error.message);
}

#[test]
fn codegen_interns_literal_without_utf8_reencoding() {
    let mut parser = Parser::from_bytes(br#"return "\000\128\255""#);
    let chunk = parser.parse().expect("byte source should parse");
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();
    let proto = CodeGenerator::new_with_pool(&mut gc, &mut pool)
        .generate(&chunk, "<byte-source-test>")
        .expect("byte source should compile");

    let literal = proto
        .constants()
        .iter()
        .find_map(|constant| match constant {
            Value::String(value) => gc
                .with_string_bytes(*value, |bytes| (bytes == [0x00, 0x80, 0xff]).then_some(()))
                .ok()
                .flatten(),
            _ => None,
        });

    assert!(
        literal.is_some(),
        "raw byte string constant was not emitted"
    );
}

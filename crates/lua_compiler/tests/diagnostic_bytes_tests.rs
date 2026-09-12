//! Lua diagnostic payloads must not be reconstructed from host-readable text.

use lua_compiler::codegen::CodegenError;
use lua_compiler::parser::{ParseError, Parser};

fn parse_error(source: &[u8]) -> ParseError {
    Parser::from_bytes(source)
        .parse()
        .expect_err("the probe must fail during parsing")
}

#[test]
fn string_constructors_keep_the_original_utf8_payload() {
    let message = "source é: expected expression";
    let parse = ParseError::new(message, 2, 7);
    let codegen = CodegenError::new(message, 2, 7);

    assert_eq!(parse.message, message);
    assert_eq!(parse.message_bytes(), message.as_bytes());
    assert_eq!(codegen.message, message);
    assert_eq!(codegen.message_bytes(), message.as_bytes());
    assert_eq!(parse.to_string(), "2:7: source é: expected expression");
    assert_eq!(codegen.to_string(), parse.to_string());
}

#[test]
fn near_token_keeps_each_high_byte_without_utf8_reencoding() {
    for byte in [0x80, 0xff] {
        let error = parse_error(&[byte]);
        let mut expected = format!("unexpected byte 0x{byte:02X} near '").into_bytes();
        expected.extend_from_slice(&[byte, b'\'']);

        assert_eq!(error.message_bytes(), expected);
        assert!(error.message.contains(&format!("\\x{byte:02x}")));
        assert!(!error.message.contains('\u{fffd}'));
        assert_eq!((error.line, error.column), (1, 1));
    }
}

#[test]
fn raw_byte_and_literal_ascii_escape_remain_distinct_diagnostics() {
    let raw = parse_error(b"'\xff'");
    let literal = parse_error(br"'\xff'");

    assert_eq!(raw.message_bytes(), b"unexpected symbol near ''\xff''");
    assert_eq!(literal.message_bytes(), br"unexpected symbol near ''\xff''");
    assert_ne!(raw.message_bytes(), literal.message_bytes());
    assert_eq!(raw.message, "unexpected symbol near ''\\xff''");
    assert_eq!(literal.message, "unexpected symbol near ''\\xff''");
}

#[test]
fn diagnostic_uses_token_spelling_for_names_numbers_and_quoted_strings() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"syntax error", b"unexpected symbol near 'error'"),
        (b"1.000", b"unexpected symbol near '1.000'"),
        (b"'aa'", b"unexpected symbol near ''aa''"),
        (b"[[a]]", b"unexpected symbol near '[[a]]'"),
    ];

    for (source, expected) in cases {
        let error = parse_error(source);
        assert_eq!(error.message_bytes(), *expected, "source: {source:?}");
        assert_eq!(error.message.as_bytes(), *expected);
    }
}

#[test]
fn eof_and_tokens_without_a_lexeme_keep_their_ascii_fallback() {
    let eof = parse_error(b"return (");
    assert!(eof.message_bytes().ends_with(b"near '<eof>'"));
    assert_eq!(eof.message.as_bytes(), eof.message_bytes());

    let empty_lexeme = parse_error(b"~");
    assert_eq!(
        empty_lexeme.message_bytes(),
        b"unexpected symbol '~' near '<error>'"
    );
}

#[test]
fn parse_to_codegen_conversion_retains_raw_bytes_and_location() {
    let parse = parse_error(b"\nreturn \xff");
    let bytes = parse.message_bytes().to_vec();
    let location = (parse.line, parse.column);
    let display = parse.to_string();
    let codegen = CodegenError::from(parse);

    assert_eq!(codegen.message_bytes(), bytes);
    assert_eq!((codegen.line, codegen.column), location);
    assert_eq!(codegen.to_string(), display);
    assert!(codegen.message.contains("\\xff"));
    assert!(!codegen.message.contains('\u{fffd}'));
}

#[test]
fn byte_constructors_escape_invalid_sequences_without_losing_valid_utf8() {
    let bytes = b"\xc3\xa9: \xff / \xe2\x82 / \0\n";
    let parse = ParseError::new_bytes(bytes.to_vec(), 3, 4);
    let codegen = CodegenError::new_bytes(bytes.to_vec(), 3, 4);

    assert_eq!(parse.message_bytes(), bytes);
    assert_eq!(codegen.message_bytes(), bytes);
    assert_eq!(parse.message, "é: \\xff / \\xe2\\x82 / \\u{0}\\n");
    assert_eq!(codegen.message, parse.message);
    assert_eq!(parse.to_string(), "3:4: é: \\xff / \\xe2\\x82 / \\u{0}\\n");
    assert_eq!(codegen.to_string(), parse.to_string());
}

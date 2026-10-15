use crate::faults;
use lua_app::{execute_multi, AppError, Session};
use lua_core::{ByteString, Value};
use lua_vm::string_format::parse_spec;

fn string_result(source: &[u8]) -> Vec<u8> {
    let values = execute_multi(source).unwrap();
    let [Value::String(bytes)] = values.as_slice() else {
        panic!("expected one actual VM string result, got {values:?}");
    };
    bytes.as_bytes().to_vec()
}

pub fn to_string_does_not_implement_default_float_precision() {
    assert_eq!(faults::float_to_string(3.5), b"3.5");
    assert_eq!(
        string_result(b"return string.format('%f',3.5)"),
        b"3.500000"
    );
    assert_eq!(string_result(b"return string.format('%.2f',3.5)"), b"3.50");
}

pub fn one_byte_specifier_loses_flags_width_and_precision() {
    let format = b"x%08.2f!";
    assert_eq!(faults::one_byte_specifier(format, 2), Some((b'0', 3)));
    let (spec, next) = parse_spec(format, 2).unwrap();
    assert!(spec.flags.zero);
    assert_eq!(spec.width, Some(8));
    assert_eq!(spec.precision, Some(2));
    assert_eq!(spec.conversion, b'f');
    assert_eq!(next, 7);
    assert_eq!(&format[next..], b"!");
    assert_eq!(
        string_result(b"return string.format('%08.2f',3.5)"),
        b"00003.50"
    );
}

pub fn stringify_accepts_a_table_that_percent_s_must_reject() {
    let mut session = Session::new();
    let table = session.execute_multi(b"return {}").unwrap().remove(0);
    assert!(!faults::stringify_any_value(&table).is_empty());
    let result = session.execute_multi(b"return string.format('%s',{})");
    assert!(matches!(result, Err(AppError::Runtime(_))));
    assert_eq!(string_result(b"return string.format('%s',42)"), b"42");
}

pub fn advancing_before_validation_commits_a_failed_argument() {
    let (spec, _) = parse_spec(b"%f", 1).unwrap();
    let values = [
        Value::String(ByteString::new(b"%f".as_slice())),
        Value::Boolean(false),
    ];
    let mut cursor = 1;
    assert!(faults::advance_before_validation(&spec, &values, &mut cursor).is_err());
    assert_eq!(
        cursor, 2,
        "the fixed fault has already skipped the invalid value"
    );
    let mut missing = values.len();
    assert!(faults::advance_before_validation(&spec, &values, &mut missing).is_err());
    assert_eq!(missing, 3);
}

// ANCHOR: string_format_real_arguments
pub fn actual_percent_escape_does_not_consume_an_argument() {
    assert_eq!(
        string_result(b"return string.format('%%:%s:%d','ok',7)"),
        b"%:ok:7"
    );
    assert_eq!(string_result(b"return string.format('%%')"), b"%");
    assert_eq!(
        string_result(b"return string.format('%%:%q',42)"),
        b"%:\"42\""
    );
    assert!(matches!(
        execute_multi(b"return string.format('%%:%s')"),
        Err(AppError::Runtime(_))
    ));
    assert!(matches!(
        execute_multi(b"return string.format('%s:%d','ok')"),
        Err(AppError::Runtime(_))
    ));
}
// ANCHOR_END: string_format_real_arguments

// ANCHOR: string_format_real_parser
pub fn actual_parser_checks_the_whole_bounded_specification() {
    let (spec, next) = parse_spec(b"%-+ #09.02fTAIL", 1).unwrap();
    assert!(spec.flags.left && spec.flags.plus && spec.flags.space);
    assert!(spec.flags.alternate && spec.flags.zero);
    assert_eq!(spec.width, Some(9));
    assert_eq!(spec.precision, Some(2));
    assert_eq!(spec.conversion, b'f');
    assert_eq!(next, 11);
    // Lua 5.1 counts flag characters; it does not reject every repeated flag.
    assert!(parse_spec(b"%++d", 1).is_ok());
    let (empty_precision, _) = parse_spec(b"%.s", 1).unwrap();
    assert_eq!(empty_precision.precision, Some(0));
    for invalid in [
        b"%100d".as_slice(),
        b"%.100f",
        b"%++++++d",
        b"%",
        b"%z",
        b"%8",
    ] {
        assert!(parse_spec(invalid, 1).is_err(), "{invalid:?}");
    }
    // Literal %% is dispatched before parse_spec, not invented as a conversion.
    assert!(parse_spec(b"%%", 1).is_err());
}
// ANCHOR_END: string_format_real_parser

pub fn actual_vm_applies_padding_precision_and_existing_conversions() {
    for (source, expected) in [
        (
            b"return string.format('%+08.2f',3.5)".as_slice(),
            b"+0003.50".as_slice(),
        ),
        (
            b"return string.format('%-6.2s','abcd')".as_slice(),
            b"ab    ".as_slice(),
        ),
        (
            b"return string.format('% d',7)".as_slice(),
            b" 7".as_slice(),
        ),
        (b"return string.format('%.0d',0)".as_slice(), b"".as_slice()),
        (
            b"return string.format('%i','-3.5')".as_slice(),
            b"-3".as_slice(),
        ),
    ] {
        assert_eq!(string_result(source), expected, "{source:?}");
    }
}

pub fn actual_failure_returns_no_partial_string_and_session_remains_usable() {
    let mut session = Session::new();
    session.execute_multi(b"saved=40").unwrap();
    assert!(matches!(
        session.execute_multi(b"return string.format('prefix:%d',{})"),
        Err(AppError::Runtime(_))
    ));
    let values = session
        .execute_multi(b"return saved+2,string.format('%s','ok')")
        .unwrap();
    assert_eq!(
        values,
        [
            Value::Number(42.0),
            Value::String(ByteString::new(b"ok".as_slice()))
        ]
    );
}

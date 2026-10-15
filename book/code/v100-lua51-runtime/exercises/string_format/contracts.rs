use crate::model::Consume;
use lua_app::Session;
use lua_core::{ByteString, Value};
use lua_vm::string_format::parse_spec;

// ANCHOR: string_format_contract
pub fn argument_cursor_commits_only_after_validation(consume: Consume) {
    for (format, value) in [
        (
            b"%08.2f".as_slice(),
            Value::String(ByteString::new(b"3.5".as_slice())),
        ),
        (b"%d".as_slice(), Value::Number(3.75)),
        (b"%i".as_slice(), Value::Number(-3.75)),
        (b"%s".as_slice(), Value::Number(42.0)),
        (b"%q".as_slice(), Value::Number(42.0)),
        (
            b"%.2s".as_slice(),
            Value::String(ByteString::new(b"abc".as_slice())),
        ),
    ] {
        let (spec, next) = parse_spec(format, 1).unwrap();
        assert_eq!(next, format.len());
        let values = [Value::String(ByteString::new(format)), value.clone()];
        let mut cursor = 1;
        assert_eq!(consume(&spec, &values, &mut cursor).unwrap(), value);
        assert_eq!(cursor, 2, "one valid conversion consumes one argument");
        let error = consume(&spec, &values, &mut cursor).unwrap_err();
        assert_eq!(error.argument, Some(3));
        assert_eq!(cursor, 2, "missing input must not commit a cursor change");
        assert_eq!(values[1], value, "borrowed argument storage stays intact");
    }
    let mut session = Session::new();
    let rejected = session.execute_multi(b"return {},nil,false").unwrap();
    for conversion in [b"%s".as_slice(), b"%q", b"%f", b"%d"] {
        let (spec, _) = parse_spec(conversion, 1).unwrap();
        for value in &rejected {
            let values = [Value::String(ByteString::new(conversion)), value.clone()];
            let mut cursor = 1;
            assert!(consume(&spec, &values, &mut cursor).is_err());
            assert_eq!(
                cursor, 1,
                "type errors leave the proposed argument unconsumed"
            );
            assert_eq!(&values[1], value);
        }
    }
}
// ANCHOR_END: string_format_contract

use lua_core::{ByteString, Value};
use lua_stdlib::HostError;

pub type NumberAdapter = fn(&[Value], usize) -> Result<f64, HostError>;
pub type BytesAdapter = fn(&[Value], usize) -> Result<Vec<u8>, HostError>;

fn string(bytes: &[u8]) -> Value {
    Value::String(ByteString::new(bytes))
}

pub fn number_rules(number: NumberAdapter) {
    assert_eq!(number(&[Value::Number(7.0)], 0), Ok(7.0));
    assert_eq!(
        number(&[Value::Boolean(false), string(b" 42.5 ")], 1),
        Ok(42.5)
    );
    assert_eq!(number(&[string(b"-1.25")], 0), Ok(-1.25));
    assert_eq!(number(&[string(b"1e2")], 0), Ok(100.0));
    assert_eq!(
        number(&[], 0),
        Err(HostError::argument(0, "value expected"))
    );
    assert_eq!(
        number(&[Value::Nil], 0),
        Err(HostError::argument(0, "number expected, got nil"))
    );
    assert_eq!(
        number(&[Value::Boolean(true)], 0),
        Err(HostError::argument(0, "number expected, got boolean"))
    );
    assert_eq!(
        number(&[Value::Number(1.0)], 2),
        Err(HostError::argument(2, "value expected"))
    );
    for bytes in [
        b"".as_slice(),
        b" ".as_slice(),
        b"wrong".as_slice(),
        b"0x10".as_slice(),
        &[255],
    ] {
        let error = number(&[Value::Number(0.0), string(bytes)], 1).unwrap_err();
        assert_eq!(error.argument, Some(2));
        assert_eq!(error.message, "number expected");
        assert_eq!(error.to_string(), "bad argument #2 (number expected)");
    }
}

pub fn bytes_rules(bytes: BytesAdapter) {
    let input = [string(&[97, 0, 255, 122])];
    let mut owned = bytes(&input, 0).unwrap();
    assert_eq!(
        owned,
        [97, 0, 255, 122],
        "binary bytes must not pass through UTF-8 replacement"
    );
    owned[0] = 88;
    let Value::String(original) = &input[0] else {
        unreachable!()
    };
    assert_eq!(
        original.as_bytes(),
        [97, 0, 255, 122],
        "owned output cannot mutate input"
    );
    assert_eq!(bytes(&[Value::Number(12.5)], 0), Ok(b"12.5".to_vec()));
    assert_eq!(
        bytes(&[Value::Boolean(false), Value::Number(7.0)], 1),
        Ok(b"7".to_vec())
    );
    assert_eq!(bytes(&[], 0), Err(HostError::argument(0, "value expected")));
    assert_eq!(
        bytes(&[Value::Nil], 0),
        Err(HostError::argument(0, "string expected, got nil"))
    );
    assert_eq!(
        bytes(&[Value::Boolean(true)], 0),
        Err(HostError::argument(0, "string expected, got boolean"))
    );
    assert_eq!(
        bytes(&[string(b"a")], 2),
        Err(HostError::argument(2, "value expected"))
    );
    let escaped = {
        let local_values = [string(&[0, 255, 1])];
        bytes(&local_values, 0).unwrap()
    };
    assert_eq!(
        escaped,
        [0, 255, 1],
        "owned Vec remains valid after argument storage drops"
    );
}

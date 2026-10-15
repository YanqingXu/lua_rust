mod contracts;
mod faults;
mod real;
mod solution;

use lua_core::{ByteString, Value};
use lua_stdlib::{Args, HostError};

#[test]
fn number_coercion_and_errors_are_shared() {
    contracts::number_rules(solution::number);
}

#[test]
fn bytes_remain_owned_and_binary_safe() {
    contracts::bytes_rules(solution::bytes);
}

#[test]
fn faulty_number_adapter_rejects_numeric_strings_and_loses_argument_index() {
    let values = [Value::String(ByteString::new(b" 42.5 ".as_slice()))];
    assert_eq!(
        faults::strict_number(&values, 0),
        Err(HostError::message("number required"))
    );
    assert_eq!(faults::strict_number(&[], 2).unwrap_err().argument, None);
}

#[test]
fn faulty_bytes_adapter_changes_binary_data_and_rejects_numbers() {
    let values = [Value::String(ByteString::new(&[97, 0, 255, 122][..]))];
    assert_eq!(
        faults::text_only_bytes(&values, 0),
        Ok(vec![97, 0, 239, 191, 189, 122])
    );
    assert!(faults::text_only_bytes(&[Value::Number(7.0)], 0).is_err());
}

#[test]
fn real_args_borrow_values_while_owned_bytes_can_escape_input_lifetime() {
    let values = [Value::Number(7.0)];
    let borrowed = real::borrowed_value(Args::new(&values), 0).unwrap();
    assert!(
        std::ptr::eq(borrowed, &values[0]),
        "Args borrows the existing Value"
    );
    assert_eq!(real::owned_bytes_after_input_drops(), [97, 0, 255, 122]);
}

#[test]
fn real_args_distinguish_required_missing_nil_and_optional_default() {
    assert_eq!(
        Args::new(&[]).required(0),
        Err(HostError::argument(0, "value expected"))
    );
    assert_eq!(Args::new(&[Value::Nil]).required(0), Ok(&Value::Nil));
    assert_eq!(Args::new(&[]).optional_integer(0, 12), Ok(12));
    assert_eq!(Args::new(&[Value::Nil]).optional_integer(0, 12), Ok(12));
    assert_eq!(
        Args::new(&[Value::Boolean(false)]).optional_integer(0, 12),
        Err(HostError::argument(0, "number expected, got boolean"))
    );
}

#[test]
fn real_args_number_parser_boundary_is_not_a_full_lua_lexer() {
    for text in [b"0x10".as_slice(), &[255]] {
        let values = [Value::String(ByteString::new(text))];
        assert_eq!(
            Args::new(&values).number(0),
            Err(HostError::argument(0, "number expected"))
        );
    }
}

#[test]
fn real_vm_math_and_string_adapters_share_conversion_rules() {
    assert_eq!(
        real::vm_coercions(),
        vec![
            Value::Number(9.0),
            Value::Number(2.0),
            Value::Number(3.0),
            Value::Number(3.0),
            Value::Number(255.0)
        ]
    );
}

#[test]
fn real_vm_maps_indexed_host_errors_to_type_errors_with_function_names() {
    use lua_vm::VmErrorKind::Type;
    assert_eq!(
        real::vm_argument_errors(),
        vec![
            (
                Type,
                "bad argument #1 (value expected) to 'math.sqrt'".to_owned()
            ),
            (
                Type,
                "bad argument #1 (number expected, got nil) to 'math.sqrt'".to_owned()
            ),
            (
                Type,
                "bad argument #2 (number expected, got boolean) to 'math.pow'".to_owned()
            ),
            (
                Type,
                "bad argument #1 (string expected, got boolean) to 'string.len'".to_owned()
            ),
        ]
    );
}

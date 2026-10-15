use lua_core::Value;
use lua_stdlib::{Args, HostError};
use lua_vm::string_format::FormatSpec;

// ANCHOR: string_format_faults
pub fn float_to_string(value: f64) -> Vec<u8> {
    value.to_string().into_bytes()
}

pub fn one_byte_specifier(format: &[u8], start: usize) -> Option<(u8, usize)> {
    format.get(start).map(|byte| (*byte, start + 1))
}

pub fn stringify_any_value(value: &Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

pub fn advance_before_validation(
    spec: &FormatSpec,
    values: &[Value],
    cursor: &mut usize,
) -> Result<Value, HostError> {
    let index = *cursor;
    *cursor += 1;
    let args = Args::new(values);
    let value = args.required(index)?;
    match spec.conversion {
        b's' | b'q' => {
            args.bytes(index)?;
        }
        _ => {
            args.number(index)?;
        }
    }
    Ok(value.clone())
}
// ANCHOR_END: string_format_faults

//! Fixed intentionally inconsistent adapters; not historical standard-library code.
use lua_core::Value;
use lua_stdlib::HostError;

pub fn strict_number(values: &[Value], index: usize) -> Result<f64, HostError> {
    match values.get(index) {
        Some(Value::Number(number)) => Ok(*number),
        _ => Err(HostError::message("number required")),
    }
}

pub fn text_only_bytes(values: &[Value], index: usize) -> Result<Vec<u8>, HostError> {
    match values.get(index) {
        Some(Value::String(bytes)) => Ok(String::from_utf8_lossy(bytes.as_bytes())
            .into_owned()
            .into_bytes()),
        _ => Err(HostError::message("string required")),
    }
}

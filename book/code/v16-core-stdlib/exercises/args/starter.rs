use lua_core::Value;
use lua_stdlib::HostError;

// ANCHOR: args_number_starter
pub fn number(values: &[Value], index: usize) -> Result<f64, HostError> {
    // TODO 1: share number coercion and indexed errors through Args.
    match values.get(index) {
        Some(Value::Number(number)) => Ok(*number),
        _ => Err(HostError::message("number required")),
    }
}
// ANCHOR_END: args_number_starter

// ANCHOR: args_bytes_starter
pub fn bytes(values: &[Value], index: usize) -> Result<Vec<u8>, HostError> {
    // TODO 2: preserve arbitrary bytes, accept numbers and share indexed errors.
    match values.get(index) {
        Some(Value::String(bytes)) => Ok(String::from_utf8_lossy(bytes.as_bytes())
            .into_owned()
            .into_bytes()),
        _ => Err(HostError::message("string required")),
    }
}
// ANCHOR_END: args_bytes_starter

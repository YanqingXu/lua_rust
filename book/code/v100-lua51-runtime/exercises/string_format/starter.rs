use lua_core::Value;
use lua_stdlib::HostError;
use lua_vm::string_format::FormatSpec;

// ANCHOR: string_format_starter
pub fn consume(
    spec: &FormatSpec,
    values: &[Value],
    cursor: &mut usize,
) -> Result<Value, HostError> {
    // TODO: validate through Args, then clone the value and advance once on success.
    let _ = (spec, values, cursor);
    Err(HostError::message("implement the argument cursor TODO"))
}
// ANCHOR_END: string_format_starter

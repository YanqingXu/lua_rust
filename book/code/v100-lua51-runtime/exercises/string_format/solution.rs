use lua_core::Value;
use lua_stdlib::{Args, HostError};
use lua_vm::string_format::FormatSpec;

// ANCHOR: string_format_solution
pub fn consume(
    spec: &FormatSpec,
    values: &[Value],
    cursor: &mut usize,
) -> Result<Value, HostError> {
    let args = Args::new(values);
    let value = args.required(*cursor)?;
    match spec.conversion {
        b's' | b'q' => {
            args.bytes(*cursor)?;
        }
        b'd' | b'i' | b'f' => {
            args.number(*cursor)?;
        }
        _ => return Err(HostError::message("unsupported conversion in exercise")),
    }
    let accepted = value.clone();
    *cursor += 1;
    Ok(accepted)
}
// ANCHOR_END: string_format_solution

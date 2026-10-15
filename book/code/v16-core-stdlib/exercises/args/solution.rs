use lua_core::Value;
use lua_stdlib::{Args, HostError};

// ANCHOR: args_number_solution
pub fn number(values: &[Value], index: usize) -> Result<f64, HostError> {
    Args::new(values).number(index)
}
// ANCHOR_END: args_number_solution

// ANCHOR: args_bytes_solution
pub fn bytes(values: &[Value], index: usize) -> Result<Vec<u8>, HostError> {
    Args::new(values).bytes(index)
}
// ANCHOR_END: args_bytes_solution

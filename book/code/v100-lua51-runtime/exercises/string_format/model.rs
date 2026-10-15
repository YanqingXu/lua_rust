use lua_core::Value;
use lua_stdlib::HostError;
use lua_vm::string_format::FormatSpec;

/// `values` includes the format string at index 0, just as the real builtin does.
pub type Consume = fn(&FormatSpec, &[Value], &mut usize) -> Result<Value, HostError>;

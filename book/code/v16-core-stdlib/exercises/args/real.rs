//! Direct probes of unchanged Args and VM; no complete Lua lexical-compatibility claim.
use lua_compiler::compile;
use lua_core::{ByteString, Value};
use lua_stdlib::{Args, HostError};
use lua_vm::{Vm, VmErrorKind};

// ANCHOR: args_borrowed_view
pub fn borrowed_value(args: Args<'_>, index: usize) -> Result<&Value, HostError> {
    args.required(index)
}
// ANCHOR_END: args_borrowed_view

// ANCHOR: args_owned_bytes
pub fn owned_bytes_after_input_drops() -> Vec<u8> {
    let local_values = [Value::String(ByteString::new(&[97, 0, 255, 122][..]))];
    Args::new(&local_values).bytes(0).unwrap()
}
// ANCHOR_END: args_owned_bytes

pub fn vm_coercions() -> Vec<Value> {
    let proto = compile(
        br#"
      local binary = string.char(97, 0, 255)
      return math.sqrt(' 81 '), math.abs('-2'), string.len(123),
        string.len(binary), string.byte(binary, 3)
    "#,
    )
    .unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}

pub fn vm_argument_errors() -> Vec<(VmErrorKind, String)> {
    [
        b"return math.sqrt()".as_slice(),
        b"return math.sqrt(nil)".as_slice(),
        b"return math.pow(2, true)".as_slice(),
        b"return string.len(false)".as_slice(),
    ]
    .into_iter()
    .map(|source| {
        let proto = compile(source).unwrap();
        let error = Vm::new().execute(&proto).unwrap_err();
        (error.kind, error.message)
    })
    .collect()
}

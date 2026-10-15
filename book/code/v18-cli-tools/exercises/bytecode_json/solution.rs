use std::fmt::Write as _;

// ANCHOR: bytecode_json_solution
pub fn encode_bytes(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    format!("{{\"type\":\"string\",\"hex\":\"{hex}\"}}")
}
// ANCHOR_END: bytecode_json_solution

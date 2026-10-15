// ANCHOR: bytecode_json_starter
pub fn encode_bytes(bytes: &[u8]) -> String {
    // TODO: retain every original byte in the snapshot's string/hex schema.
    format!(
        "{{\"type\":\"string\",\"value\":\"{}\"}}",
        String::from_utf8_lossy(bytes)
    )
}
// ANCHOR_END: bytecode_json_starter

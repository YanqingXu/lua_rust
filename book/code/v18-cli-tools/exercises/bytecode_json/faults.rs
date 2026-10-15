// ANCHOR: bytecode_json_raw_fault
pub fn raw_text(bytes: &[u8]) -> String {
    // Valid UTF-8 with quotes/control bytes already breaks this unescaped object.
    format!(
        "{{\"type\":\"string\",\"value\":\"{}\"}}",
        String::from_utf8_lossy(bytes)
    )
}
// ANCHOR_END: bytecode_json_raw_fault

pub fn lossy_text(bytes: &[u8]) -> String {
    // For the invalid single-byte collision fixture this is syntactically JSON,
    // but different Lua byte strings become the same Unicode replacement text.
    format!(
        "{{\"type\":\"string\",\"value\":\"{}\"}}",
        String::from_utf8_lossy(bytes)
    )
}

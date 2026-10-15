//! Narrow view schema, not a JSON parser or Proto deserializer.
pub type Encode = fn(&[u8]) -> String;

pub fn cases() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("empty", vec![]),
        ("ascii", b"Lua".to_vec()),
        ("quotes-slashes", b"\"\\/".to_vec()),
        ("controls", vec![0, 9, 10, 13, 31]),
        ("invalid-utf8", vec![255, 254, 128]),
        ("utf8-bytes", "中文".as_bytes().to_vec()),
        ("all-bytes", (0..=255).collect()),
    ]
}

// ANCHOR: bytecode_json_decode
pub fn decode_constant(json: &str) -> Result<Vec<u8>, String> {
    let hex = json
        .strip_prefix("{\"type\":\"string\",\"hex\":\"")
        .and_then(|value| value.strip_suffix("\"}"))
        .ok_or("expected exactly string/hex view object")?;
    if hex.len() % 2 != 0
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err("hex must have an even number of lowercase ASCII digits".to_owned());
    }
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}
// ANCHOR_END: bytecode_json_decode

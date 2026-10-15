use super::model::{cases, decode_constant, Encode};

// ANCHOR: bytecode_json_contract
pub fn exact_bytes(encode: Encode) {
    for (label, bytes) in cases() {
        let encoded = encode(&bytes);
        assert_eq!(decode_constant(&encoded), Ok(bytes), "{label}: {encoded:?}");
    }
    assert_ne!(
        encode(&[255]),
        encode(&[254]),
        "invalid UTF-8 byte strings are distinct"
    );
}
// ANCHOR_END: bytecode_json_contract

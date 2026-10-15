#[path = "../support/bytecode_process.rs"]
mod bytecode_process;
mod contracts;
mod faults;
mod model;
#[path = "../support/process.rs"]
mod process;
mod real;
mod solution;
#[path = "../support/temp.rs"]
mod temp;

#[test]
fn bytes_constants_preserve_exact_bytes_as_lower_hex() {
    contracts::exact_bytes(solution::encode_bytes);
}

#[test]
fn raw_text_injects_quotes_backslashes_and_control_bytes() {
    let raw = faults::raw_text(b"\"\\\0\n");
    assert_eq!(raw, "{\"type\":\"string\",\"value\":\"\"\\\0\n\"}");
    assert!(raw.bytes().any(|byte| byte < 32));
    assert!(model::decode_constant(&raw).is_err());
}

#[test]
fn lossy_conversion_merges_distinct_invalid_utf8_strings() {
    assert_ne!([255], [254]);
    let left = faults::lossy_text(&[255]);
    let right = faults::lossy_text(&[254]);
    assert_eq!(left, right);
    assert_eq!(left, "{\"type\":\"string\",\"value\":\"�\"}");
    assert_ne!(
        solution::encode_bytes(&[255]),
        solution::encode_bytes(&[254])
    );
}

#[test]
fn real_proto_json_matches_bytes_nested_nonfinite_and_raw_code_schema() {
    assert_eq!(real::document(), real::expected_document());
}

#[test]
fn real_metadata_escape_preserves_byte_spelling_not_unicode_scalar_text() {
    let mut proto = real::fixture();
    proto.debug_name = "é".to_owned();
    let json = lua_bytecode::proto_json(&proto);
    assert!(json.starts_with("{\"name\":\"\\u00c3\\u00a9\","));
    assert!(!json.starts_with("{\"name\":\"é\","));
}

#[test]
fn real_bytecode_binary_stdin_and_file_equal_public_renderer() {
    let proto = lua_compiler::compile(real::SOURCE).unwrap();
    assert_eq!(
        proto.protos[0].constants[0],
        lua_core::Value::String(lua_core::ByteString::new(vec![0, 255, 34, 92, 47, 10]))
    );
    let probes =
        real::binary_documents(std::path::Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0].mode, "stdin");
    assert_eq!(probes[1].mode, "file");
    assert_eq!(probes[0].stdout, probes[1].stdout);
}

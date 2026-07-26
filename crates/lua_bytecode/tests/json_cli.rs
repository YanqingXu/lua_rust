use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn json_cli_emits_recursive_byte_evidence() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    let input = std::env::temp_dir().join(format!(
        "lua-bytecode-json-{}-{nonce}.lua",
        std::process::id()
    ));

    let mut source = b"local captured = \"bytes".to_vec();
    source.extend_from_slice(&[0, 0xff]);
    source.extend_from_slice(b"\"\nreturn function(argument)\n  return captured, argument\nend\n");
    fs::write(&input, source).expect("temporary Lua source should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lua_bytecode"))
        .arg(&input)
        .arg("--format=json")
        .output()
        .expect("lua_bytecode should start");
    fs::remove_file(&input).expect("temporary Lua source should be removable");

    assert!(
        output.status.success(),
        "lua_bytecode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "successful JSON output should not emit stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("CLI stdout should be one JSON document");
    assert_eq!(document["schema_version"], 2);
    assert_eq!(document["child_count"], 1);
    assert_eq!(document["sub_protos"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["local_names"][0]["bytes"], "6361707475726564");

    let string_constant = document["constants"]
        .as_array()
        .expect("constants should be an array")
        .iter()
        .find(|constant| constant["type"] == "string")
        .expect("root Proto should contain its byte string constant");
    assert_eq!(string_constant["value"]["encoding"], "hex");
    assert_eq!(string_constant["value"]["bytes"], "627974657300ff");
    assert_eq!(
        document["sub_protos"][0]["upvalue_names"][0]["bytes"],
        "6361707475726564"
    );
}

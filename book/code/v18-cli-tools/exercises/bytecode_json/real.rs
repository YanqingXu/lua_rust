//! Actual Proto renderer and actual bytecode process, without replacing either implementation.
use lua_bytecode::proto_json;
use lua_compiler::compile;
use lua_core::{ByteString, Instruction, Opcode, Proto, Value};
use std::path::Path;
use std::rc::Rc;

use crate::{bytecode_process, process, temp::TempDirectory};

fn empty_proto(name: &str, constants: Vec<Value>) -> Proto {
    Proto {
        constants,
        code: vec![],
        lines: vec![],
        max_stack_size: 1,
        protos: vec![],
        num_params: 0,
        is_vararg: false,
        upvalues: vec![],
        debug_name: name.to_owned(),
    }
}

// ANCHOR: bytecode_json_real_fixture
pub fn fixture() -> Proto {
    let mut proto = empty_proto(
        "root\"\\/\n\r\t\0é",
        vec![
            Value::Nil,
            Value::Boolean(true),
            Value::Number(1.25),
            Value::Number(f64::NAN),
            Value::Number(f64::INFINITY),
            Value::Number(f64::NEG_INFINITY),
        ],
    );
    proto.constants.extend(
        super::model::cases()
            .into_iter()
            .map(|(_, bytes)| Value::String(ByteString::new(bytes))),
    );
    proto.code = vec![
        Instruction::abc(Opcode::Move, 7, 9, 11),
        Instruction::from_raw(u32::MAX),
    ];
    proto.lines = vec![71]; // The view falls back to line 0 for the second word.
    proto.max_stack_size = 8;
    proto.num_params = 2;
    proto.is_vararg = true;
    proto.protos.push(Rc::new(empty_proto(
        "nested",
        vec![Value::String(ByteString::new(vec![0, 255]))],
    )));
    proto
}
// ANCHOR_END: bytecode_json_real_fixture

pub fn document() -> String {
    proto_json(&fixture())
}

pub fn expected_document() -> String {
    let mut constants = vec![
        "{\"type\":\"nil\"}".to_owned(),
        "{\"type\":\"boolean\",\"value\":true}".to_owned(),
        "{\"type\":\"number\",\"value\":1.25}".to_owned(),
        "{\"type\":\"number\",\"value\":\"NaN\"}".to_owned(),
        "{\"type\":\"number\",\"value\":\"inf\"}".to_owned(),
        "{\"type\":\"number\",\"value\":\"-inf\"}".to_owned(),
    ];
    constants.extend(
        super::model::cases()
            .into_iter()
            .map(|(_, bytes)| super::solution::encode_bytes(&bytes)),
    );
    format!("{{\"name\":\"root\\\"\\\\/\\n\\r\\t\\u0000\\u00c3\\u00a9\",\"max_stack\":8,\"params\":2,\"vararg\":true,\"constants\":[{}],\"code\":[{{\"pc\":0,\"line\":71,\"raw\":75678144,\"op\":\"Move\",\"a\":7,\"b\":9,\"c\":11,\"bx\":4619,\"sbx\":-126452}},{{\"pc\":1,\"line\":0,\"raw\":4294967295,\"op\":\"INVALID_63\",\"a\":255,\"b\":511,\"c\":511,\"bx\":262143,\"sbx\":131072}}],\"protos\":[{{\"name\":\"nested\",\"max_stack\":1,\"params\":0,\"vararg\":false,\"constants\":[{{\"type\":\"string\",\"hex\":\"00ff\"}}],\"code\":[],\"protos\":[]}}]}}\n",constants.join(","))
}

pub const SOURCE: &[u8] =
    br#"local inner=function() return '\000\255\"\\/\010' end; return inner()"#;

pub struct ProcessProbe {
    pub mode: &'static str,
    pub stdout: Vec<u8>,
}

// ANCHOR: bytecode_json_process
#[allow(dead_code)] // Integration entry; examples already have a bytecode binary path.
pub fn binary_documents(lua_app: &Path) -> Result<Vec<ProcessProbe>, String> {
    let binary = bytecode_process::sibling_of_lua_app(lua_app)?;
    binary_documents_at(&binary)
}

pub fn binary_documents_at(binary: &Path) -> Result<Vec<ProcessProbe>, String> {
    let directory = TempDirectory::new("bytecode-json")?;
    let result = (|| {
        let path = directory.path("fixture.lua");
        std::fs::write(&path, SOURCE).map_err(|error| error.to_string())?;
        let expected = proto_json(&compile(SOURCE).map_err(|error| error.to_string())?);
        let mut probes = Vec::new();
        for (mode, args, input) in [
            (
                "stdin",
                vec!["--format=json".to_owned(), "-".to_owned()],
                SOURCE,
            ),
            (
                "file",
                vec![
                    "--format".to_owned(),
                    "json".to_owned(),
                    path.to_string_lossy().into_owned(),
                ],
                &b""[..],
            ),
        ] {
            let output = process::run(binary, &args, input)?;
            if output.status.code() != Some(0)
                || !output.stderr.is_empty()
                || output.stdout != expected.as_bytes()
            {
                return Err(format!("{mode}: unexpected bytecode output {output:?}"));
            }
            probes.push(ProcessProbe {
                mode,
                stdout: output.stdout,
            });
        }
        Ok(probes)
    })();
    directory.finish(result)
}
// ANCHOR_END: bytecode_json_process

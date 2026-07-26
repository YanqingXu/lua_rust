//! lua_bytecode — Lua 5.1 bytecode dumper
//!
//! Compiles Lua source files and displays their bytecode in readable format.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::value::Value;
use serde_json::{Value as JsonValue, json};

use std::env;
use std::fs;
use std::io::{self, Write};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: lua_bytecode <file.lua> [--format=text|json]");
        eprintln!("  Compiles Lua source and displays bytecode.");
        return;
    }

    let filename = &args[1];
    let mut format = "text";
    for arg in &args[2..] {
        if let Some(f) = arg.strip_prefix("--format=") {
            format = f;
        }
    }

    match dump_file(filename, format) {
        Ok(_) => {}
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn dump_file(filename: &str, format: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = read_lua_source_file(filename)?;

    // Compile with the same explicit string-interning service used by the
    // runtime. The pool stays alive while the resulting Proto is inspected.
    let mut temp_gc = GarbageCollector::new();
    let mut temp_pool = StringPool::new();
    let proto = compile_lua_source(&source, filename, &mut temp_gc, &mut temp_pool)?;

    match format {
        "json" => dump_json(&proto, filename, &temp_gc)?,
        _ => dump_text(&proto, filename, &source),
    }

    Ok(())
}

fn compile_lua_source(
    source: &[u8],
    source_name: &str,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
) -> Result<Proto, Box<dyn std::error::Error>> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse()?;
    Ok(CodeGenerator::new_with_pool(gc, string_pool).generate(&chunk, source_name)?)
}

fn read_lua_source_file(filename: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(fs::read(filename)?)
}

fn dump_text(proto: &lua_core::proto::Proto, filename: &str, source: &[u8]) {
    println!("=== Lua Bytecode: {} ===", filename);
    println!("Source size: {} bytes", source.len());
    println!(
        "Instructions: {} | Constants: {} | Sub-protos: {}",
        proto.instruction_count(),
        proto.constant_count(),
        proto.sub_proto_count()
    );
    println!(
        "Params: {} | Vararg: {} | Max Stack: {}",
        proto.num_params(),
        proto.vararg_flags(),
        proto.max_stack_size()
    );
    println!();

    // Print constants
    if proto.constant_count() > 0 {
        println!("Constants:");
        for (i, c) in proto.constants().iter().enumerate() {
            println!("  [{}] {}", i, format_constant(c));
        }
        println!();
    }

    // Print instructions with source lines
    if proto.instruction_count() > 0 {
        println!("Bytecode:");
        println!("{:<6} {:<4} {:<10} Args", "PC", "Line", "Opcode");
        println!("{}", "-".repeat(60));

        let code = proto.code();
        for (pc, &inst) in code.iter().enumerate() {
            let op = opcode::get_opcode(inst);
            let line = if pc < proto.line_info().len() {
                proto.line_info()[pc]
            } else {
                0
            };

            let args = format_instruction_args(op, inst);
            let source_line = source_line_for_display(source, line);

            println!(
                "{:<6} {:<4} {:<10} {}",
                pc,
                line,
                opcode::get_op_name(op),
                args
            );
            if !source_line.is_empty() && pc == 0
                || (pc > 0 && {
                    let prev_line = if pc > 0 && pc <= proto.line_info().len() {
                        proto.line_info()[pc - 1]
                    } else {
                        -1
                    };
                    line != prev_line
                })
            {
                println!("              ; {}", source_line);
            }
        }
        println!();
    }
}

fn source_line_for_display(source: &[u8], line: i32) -> String {
    if line <= 0 {
        return String::new();
    }
    let Some(line) = source.split(|byte| *byte == b'\n').nth((line - 1) as usize) else {
        return String::new();
    };
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    String::from_utf8_lossy(&line[start..end]).into_owned()
}

fn dump_json(
    proto: &Proto,
    filename: &str,
    gc: &GarbageCollector,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut document = proto_json(proto, gc, &mut Vec::new())?;
    let object = document.as_object_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Proto JSON serializer did not produce an object",
        )
    })?;
    object.insert("schema_version".to_string(), JsonValue::from(2));
    object.insert("input".to_string(), JsonValue::from(filename));

    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &document)?;
    writeln!(output)?;
    Ok(())
}

fn proto_json(
    proto: &Proto,
    gc: &GarbageCollector,
    ancestry: &mut Vec<lua_core::gc::object_id::ObjectId>,
) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let source = match proto.source() {
        Some(source) => gc.with_ref(source, |source| byte_envelope(source.as_bytes()))?,
        None => JsonValue::Null,
    };

    let mut constants = Vec::with_capacity(proto.constant_count());
    for constant in proto.constants() {
        constants.push(constant_json(constant, gc)?);
    }

    let mut instructions = Vec::with_capacity(proto.instruction_count());
    let mut line_info = Vec::with_capacity(proto.instruction_count());
    for (pc, &instruction) in proto.code().iter().enumerate() {
        let op = opcode::get_opcode(instruction);
        let line = proto.line_info().get(pc).copied();
        line_info.push(line);
        instructions.push(json!({
            "pc": pc,
            "line": line,
            "op": opcode::get_op_name(op),
            "a": opcode::get_arg_a(instruction),
            "b": opcode::get_arg_b(instruction),
            "c": opcode::get_arg_c(instruction),
            "bx": opcode::get_arg_bx(instruction),
            "sbx": opcode::get_arg_sbx(instruction),
        }));
    }

    let mut local_names = Vec::with_capacity(proto.loc_var_count());
    let mut locals = Vec::with_capacity(proto.loc_var_count());
    for index in 0..proto.loc_var_count() {
        let local = proto.loc_var(index);
        let name = match local.varname {
            Some(name) => gc.with_ref(name, |name| byte_envelope(name.as_bytes()))?,
            None => JsonValue::Null,
        };
        local_names.push(name.clone());
        locals.push(json!({
            "name": name,
            "start_pc": local.startpc,
            "end_pc": local.endpc,
            "register": local.reg,
        }));
    }

    let upvalue_count = usize::from(proto.num_upvalues()).max(proto.upvalue_name_count());
    let mut upvalue_names = Vec::with_capacity(upvalue_count);
    for index in 0..upvalue_count {
        let name = match proto.upvalue_name(index) {
            Some(name) => gc.with_ref(name, |name| byte_envelope(name.as_bytes()))?,
            None => JsonValue::Null,
        };
        upvalue_names.push(name);
    }

    let mut sub_protos = Vec::with_capacity(proto.sub_proto_count());
    for index in 0..proto.sub_proto_count() {
        let child_ref = proto.sub_proto(index);
        let object_id = child_ref.object_id();
        if ancestry.contains(&object_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Proto graph contains a cycle through {object_id:?}"),
            )
            .into());
        }

        ancestry.push(object_id);
        let child_result = gc.with_ref(child_ref, |child| proto_json(child, gc, ancestry));
        ancestry.pop();
        sub_protos.push(child_result??);
    }

    Ok(json!({
        "source": source,
        "line_defined": proto.line_defined(),
        "last_line_defined": proto.last_line_defined(),
        "params": proto.num_params(),
        "vararg": proto.vararg_flags(),
        "max_stack": proto.max_stack_size(),
        "num_upvalues": proto.num_upvalues(),
        "child_count": proto.sub_proto_count(),
        "constants": constants,
        "instructions": instructions,
        "line_info": line_info,
        "line_info_entry_count": proto.line_info().len(),
        "local_names": local_names,
        "locals": locals,
        "upvalue_names": upvalue_names,
        "sub_protos": sub_protos,
    }))
}

fn constant_json(
    value: &Value,
    gc: &GarbageCollector,
) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let result = match value {
        Value::Nil => json!({"type": "nil"}),
        Value::Boolean(value) => json!({"type": "boolean", "value": value}),
        Value::Number(value) => {
            let json_number = serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null);
            json!({
                "type": "number",
                "value": json_number,
                "bits": format!("{:016x}", value.to_bits()),
            })
        }
        Value::String(value) => {
            let encoded = gc.with_ref(*value, |value| byte_envelope(value.as_bytes()))?;
            json!({"type": "string", "value": encoded})
        }
        Value::Table(_) => json!({"type": "table", "value": null}),
        Value::Function(_) => json!({"type": "function", "value": null}),
        Value::Userdata(_) => json!({"type": "userdata", "value": null}),
        Value::Thread(_) => json!({"type": "thread", "value": null}),
        Value::LightUserdata(_) => json!({"type": "lightuserdata", "value": null}),
    };
    Ok(result)
}

fn byte_envelope(bytes: &[u8]) -> JsonValue {
    json!({
        "encoding": "hex",
        "bytes": bytes_to_hex(bytes),
        "byte_length": bytes.len(),
    })
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn format_instruction_args(op: OpCode, inst: lua_core::proto::Instruction) -> String {
    match opcode::get_op_mode(op) {
        opcode::OpMode::IABC => {
            format!(
                "A={} B={} C={}",
                opcode::get_arg_a(inst),
                opcode::get_arg_b(inst),
                opcode::get_arg_c(inst)
            )
        }
        opcode::OpMode::IABx => {
            format!(
                "A={} Bx={}",
                opcode::get_arg_a(inst),
                opcode::get_arg_bx(inst)
            )
        }
        opcode::OpMode::IAsBx => {
            format!(
                "A={} sBx={}",
                opcode::get_arg_a(inst),
                opcode::get_arg_sbx(inst)
            )
        }
    }
}

fn format_constant(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => format!("bool: {}", b),
        Value::Number(n) => format!("number: {}", n),
        Value::String(_) => "string".to_string(),
        Value::Table(_) => "table".to_string(),
        Value::Function(_) => "function".to_string(),
        Value::Userdata(_) => "userdata".to_string(),
        Value::Thread(_) => "thread".to_string(),
        Value::LightUserdata(_) => "lightuserdata".to_string(),
    }
}

#[cfg(test)]
mod byte_source_tests {
    use super::*;

    #[test]
    fn non_utf8_literal_reaches_proto_without_reencoding() {
        let mut source = b"return \"".to_vec();
        source.extend_from_slice(&[0x00, 0x80, 0xff]);
        source.push(b'"');

        let mut parser = Parser::from_bytes(&source);
        let chunk = parser.parse().expect("byte source should parse");
        let mut gc = GarbageCollector::new();
        let proto = CodeGenerator::new(&mut gc)
            .generate(&chunk, "@bytes.lua")
            .expect("byte source should compile");
        let string = proto
            .constants()
            .iter()
            .find_map(|value| match value {
                Value::String(string) => Some(*string),
                _ => None,
            })
            .expect("literal should become a string constant");
        // SAFETY: `gc` remains live for the duration of this assertion.
        let string = unsafe { string.as_ref() }.expect("constant must be non-null");

        assert_eq!(string.as_bytes(), &[0x00, 0x80, 0xff]);
    }

    #[test]
    fn source_line_display_is_explicitly_lossy_only_at_output_boundary() {
        assert_eq!(
            source_line_for_display(b" first\n \xffsecond \r\n", 2),
            "\u{fffd}second"
        );
    }

    #[test]
    fn cli_compilation_interns_repeated_string_constants() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let proto = compile_lua_source(
            b"print(1)\nprint(2)",
            "@interned-constants.lua",
            &mut gc,
            &mut pool,
        )
        .expect("source should compile");

        assert_eq!(proto.constant_count(), 3);
        let globals = proto
            .code()
            .iter()
            .copied()
            .filter(|inst| opcode::get_opcode(*inst) == OpCode::GETGLOBAL)
            .collect::<Vec<_>>();
        assert_eq!(globals.len(), 2);
        assert_eq!(opcode::get_arg_bx(globals[0]), 0);
        assert_eq!(opcode::get_arg_bx(globals[1]), 0);

        let Value::String(print_name) = proto.constant(0) else {
            panic!("first constant should be the interned global name");
        };
        // SAFETY: `gc` remains alive and no collection runs during this read.
        let print_name = unsafe { print_name.as_ref() }.expect("string constant should be live");
        assert_eq!(print_name.as_bytes(), b"print");
    }

    #[test]
    fn json_evidence_is_recursive_and_byte_safe() {
        let mut source = b"local outer = \"".to_vec();
        source.extend_from_slice(&[b'a', 0, 0xff]);
        source.extend_from_slice(b"\"\nreturn function(argument)\n  return outer, argument\nend\n");

        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let proto = compile_lua_source(&source, "@json-evidence.lua", &mut gc, &mut pool)
            .expect("nested byte source should compile");
        let evidence =
            proto_json(&proto, &gc, &mut Vec::new()).expect("Proto tree should serialize");

        assert_eq!(evidence["child_count"], 1);
        assert_eq!(evidence["sub_protos"].as_array().map(Vec::len), Some(1));
        assert_eq!(evidence["line_defined"], 0);
        assert_eq!(evidence["last_line_defined"], 0);
        assert_eq!(evidence["local_names"][0]["bytes"], bytes_to_hex(b"outer"));

        let string_constant = evidence["constants"]
            .as_array()
            .expect("constants should be an array")
            .iter()
            .find(|constant| constant["type"] == "string")
            .expect("root Proto should contain the byte string constant");
        assert_eq!(string_constant["value"]["encoding"], "hex");
        assert_eq!(string_constant["value"]["bytes"], "6100ff");
        assert_eq!(string_constant["value"]["byte_length"], 3);

        let child = &evidence["sub_protos"][0];
        assert!(child["line_defined"].as_i64().is_some());
        assert!(child["last_line_defined"].as_i64().is_some());
        assert_eq!(child["upvalue_names"][0]["bytes"], bytes_to_hex(b"outer"));
        assert_eq!(child["local_names"][0]["bytes"], bytes_to_hex(b"argument"));
        assert_eq!(
            child["line_info"].as_array().map(Vec::len),
            child["instructions"].as_array().map(Vec::len)
        );

        let rendered =
            serde_json::to_string_pretty(&evidence).expect("evidence should be valid JSON");
        let reparsed: JsonValue =
            serde_json::from_str(&rendered).expect("evidence should round-trip as JSON");
        assert_eq!(reparsed, evidence);
    }

    #[test]
    fn json_evidence_uses_null_for_missing_debug_data() {
        let mut proto = Proto::new();
        proto.set_num_upvalues(1);
        proto.add_instruction(opcode::create_abc(OpCode::RETURN, 0, 1, 0));

        let gc = GarbageCollector::new();
        let evidence =
            proto_json(&proto, &gc, &mut Vec::new()).expect("minimal Proto should serialize");

        assert_eq!(evidence["upvalue_names"][0], JsonValue::Null);
        assert_eq!(evidence["instructions"][0]["line"], JsonValue::Null);
        assert_eq!(
            evidence["line_info"].as_array().map(Vec::len),
            Some(1),
            "missing line information must not be synthesized as line zero"
        );
        assert_eq!(evidence["line_info"][0], JsonValue::Null);
        assert_eq!(evidence["line_info_entry_count"], 0);
    }
}

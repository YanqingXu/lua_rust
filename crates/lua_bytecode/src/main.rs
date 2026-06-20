//! lua_bytecode — Lua 5.1 bytecode dumper
//!
//! Compiles Lua source files and displays their bytecode in readable format.
//! C++ reference: `lua_cpp/src/bytecode/`.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::value::Value;

use std::env;
use std::fs;

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
    let source = fs::read_to_string(filename)?;

    // Parse
    let mut parser = Parser::new(&source);
    let chunk = parser.parse()?;

    // Compile (create temporary GC for string constant allocation)
    let mut temp_gc = GarbageCollector::new();
    let cg = CodeGenerator::new(&mut temp_gc);
    let proto = cg.generate(&chunk, filename)?;

    match format {
        "json" => dump_json(&proto, filename),
        _ => dump_text(&proto, filename, &source),
    }

    Ok(())
}

fn dump_text(proto: &lua_core::proto::Proto, filename: &str, source: &str) {
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
            let source_line = if line > 0 && (line as usize) <= source.lines().count() {
                source.lines().nth((line - 1) as usize).unwrap_or("").trim()
            } else {
                ""
            };

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

fn dump_json(proto: &lua_core::proto::Proto, filename: &str) {
    println!("{{");
    println!("  \"source\": \"{}\",", filename);
    println!("  \"params\": {},", proto.num_params());
    println!("  \"vararg\": {},", proto.vararg_flags());
    println!("  \"max_stack\": {},", proto.max_stack_size());
    println!("  \"constants\": [");
    for (i, c) in proto.constants().iter().enumerate() {
        let comma = if i + 1 < proto.constant_count() {
            ","
        } else {
            ""
        };
        println!("    {}{}", format_constant_json(c), comma);
    }
    println!("  ],");
    println!("  \"instructions\": [");
    let code = proto.code();
    for (pc, &inst) in code.iter().enumerate() {
        let op = opcode::get_opcode(inst);
        let line = if pc < proto.line_info().len() {
            proto.line_info()[pc]
        } else {
            0
        };
        let comma = if pc + 1 < code.len() { "," } else { "" };
        println!(
            "    {{\"pc\": {}, \"line\": {}, \"op\": \"{}\", \"a\": {}, \"b\": {}, \"c\": {}, \"bx\": {}, \"sbx\": {}}}{}",
            pc,
            line,
            opcode::get_op_name(op),
            opcode::get_arg_a(inst),
            opcode::get_arg_b(inst),
            opcode::get_arg_c(inst),
            opcode::get_arg_bx(inst),
            opcode::get_arg_sbx(inst),
            comma
        );
    }
    println!("  ]");
    println!("}}");
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

fn format_constant_json(v: &Value) -> String {
    match v {
        Value::Nil => r#"{"type": "nil"}"#.to_string(),
        Value::Boolean(b) => format!(r#"{{"type": "boolean", "value": {}}}"#, b),
        Value::Number(n) => format!(r#"{{"type": "number", "value": {}}}"#, n),
        Value::String(_) => r#"{"type": "string"}"#.to_string(),
        _ => r#"{"type": "other"}"#.to_string(),
    }
}

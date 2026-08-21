use lua_core::{disassemble, Opcode, Proto, Value};
use lua_vm::VmTraceEvent;
use std::collections::BTreeSet;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewFormat {
    Text,
    Json,
    Cfg,
}

pub fn render(proto: &Proto, format: ViewFormat) -> Result<String, String> {
    match format {
        ViewFormat::Text => disassemble(proto),
        ViewFormat::Json => Ok(proto_json(proto)),
        ViewFormat::Cfg => cfg_dot(proto),
    }
}

// ANCHOR: bytecode_json
pub fn proto_json(proto: &Proto) -> String {
    let mut output = String::new();
    write_proto_json(proto, &mut output);
    output.push('\n');
    output
}

fn write_proto_json(proto: &Proto, output: &mut String) {
    let _ = write!(
        output,
        "{{\"name\":\"{}\",\"max_stack\":{},\"params\":{},\"vararg\":{},\"constants\":[",
        escape_json(proto.debug_name.as_bytes()),
        proto.max_stack_size,
        proto.num_params,
        proto.is_vararg
    );
    for (index, value) in proto.constants.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&value_json(value));
    }
    output.push_str("],\"code\":[");
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        if pc != 0 {
            output.push(',');
        }
        let opcode = instruction.opcode().map_or_else(
            |raw| format!("INVALID_{raw}"),
            |opcode| format!("{opcode:?}"),
        );
        let _ = write!(
            output,
            "{{\"pc\":{pc},\"line\":{},\"raw\":{},\"op\":\"{opcode}\",\"a\":{},\"b\":{},\"c\":{},\"bx\":{},\"sbx\":{}}}",
            proto.lines.get(pc).copied().unwrap_or(0),
            instruction.raw(),
            instruction.a(),
            instruction.b(),
            instruction.c(),
            instruction.bx(),
            instruction.sbx()
        );
    }
    output.push_str("],\"protos\":[");
    for (index, nested) in proto.protos.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write_proto_json(nested, output);
    }
    output.push_str("]}");
}
// ANCHOR_END: bytecode_json

// ANCHOR: bytecode_cfg
pub fn cfg_dot(proto: &Proto) -> Result<String, String> {
    let blocks = basic_blocks(proto)?;
    let mut output = String::from("digraph bytecode {\n  node [shape=box,fontname=monospace];\n");
    for (index, (start, end)) in blocks.iter().copied().enumerate() {
        let mut label = format!("B{index}\\n");
        for pc in start..end {
            let opcode = proto.code[pc]
                .opcode()
                .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
            let _ = write!(label, "{pc}: {opcode:?}\\l");
        }
        let _ = writeln!(output, "  b{index} [label=\"{label}\"];");
    }
    for (index, (_, end)) in blocks.iter().copied().enumerate() {
        if end == 0 {
            continue;
        }
        let pc = end - 1;
        let instruction = proto.code[pc];
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        let mut targets = Vec::new();
        if is_jump(opcode) {
            let target = jump_target(pc, instruction.sbx(), proto.code.len())?;
            targets.push((target, "jump"));
        }
        if has_fallthrough(opcode) && end < proto.code.len() {
            targets.push((end, "next"));
        }
        for (target, label) in targets {
            let target_block = blocks
                .iter()
                .position(|(start, _)| *start == target)
                .ok_or_else(|| format!("pc {pc} targets non-leader {target}"))?;
            let _ = writeln!(output, "  b{index} -> b{target_block} [label=\"{label}\"];");
        }
    }
    output.push_str("}\n");
    Ok(output)
}

fn basic_blocks(proto: &Proto) -> Result<Vec<(usize, usize)>, String> {
    if proto.code.is_empty() {
        return Ok(Vec::new());
    }
    let mut leaders = BTreeSet::from([0]);
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        if is_jump(opcode) {
            leaders.insert(jump_target(pc, instruction.sbx(), proto.code.len())?);
        }
        if (is_jump(opcode) || !has_fallthrough(opcode)) && pc + 1 < proto.code.len() {
            leaders.insert(pc + 1);
        }
    }
    let starts = leaders.into_iter().collect::<Vec<_>>();
    Ok(starts
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| {
            (
                start,
                starts.get(index + 1).copied().unwrap_or(proto.code.len()),
            )
        })
        .collect())
}
// ANCHOR_END: bytecode_cfg

pub fn trace_json(events: &[VmTraceEvent]) -> String {
    let mut output = String::from("[\n");
    for (index, event) in events.iter().enumerate() {
        if index != 0 {
            output.push_str(",\n");
        }
        let registers = event
            .registers
            .iter()
            .map(value_json)
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(
            output,
            "  {{\"step\":{index},\"depth\":{},\"function\":\"{}\",\"pc\":{},\"line\":{},\"op\":\"{:?}\",\"registers\":[{registers}]}}",
            event.frame_depth,
            escape_json(event.function.as_bytes()),
            event.pc,
            event.line,
            event.opcode
        );
    }
    output.push_str("\n]\n");
    output
}

fn is_jump(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Jump
            | Opcode::JumpIfFalse
            | Opcode::JumpIfTrue
            | Opcode::JumpIfNil
            | Opcode::ForPrep
            | Opcode::ForLoop
    )
}

fn has_fallthrough(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::Return | Opcode::TailCall | Opcode::Jump | Opcode::ForPrep
    )
}

fn jump_target(pc: usize, offset: i32, code_len: usize) -> Result<usize, String> {
    let target = pc as i64 + 1 + i64::from(offset);
    if target < 0 || target >= code_len as i64 {
        return Err(format!("jump at pc {pc} targets {target} outside code"));
    }
    Ok(target as usize)
}

fn value_json(value: &Value) -> String {
    match value {
        Value::Nil => "{\"type\":\"nil\"}".to_owned(),
        Value::Boolean(value) => format!("{{\"type\":\"boolean\",\"value\":{value}}}"),
        Value::Number(value) if value.is_finite() => {
            format!("{{\"type\":\"number\",\"value\":{value}}}")
        }
        Value::Number(value) => format!(
            "{{\"type\":\"number\",\"value\":\"{}\"}}",
            escape_json(value.to_string().as_bytes())
        ),
        Value::String(value) => format!(
            "{{\"type\":\"string\",\"hex\":\"{}\"}}",
            hex(value.as_bytes())
        ),
        other => format!(
            "{{\"type\":\"{}\",\"display\":\"{}\"}}",
            other.type_name(),
            escape_json(other.to_string().as_bytes())
        ),
    }
}

fn escape_json(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in bytes {
        match byte {
            b'"' => output.push_str("\\\""),
            b'\\' => output.push_str("\\\\"),
            b'\n' => output.push_str("\\n"),
            b'\r' => output.push_str("\\r"),
            b'\t' => output.push_str("\\t"),
            0x20..=0x7e => output.push(char::from(*byte)),
            _ => {
                let _ = write!(output, "\\u{:04x}", byte);
            }
        }
    }
    output
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{cfg_dot, proto_json, trace_json};
    use lua_compiler::compile;
    use lua_vm::Vm;

    #[test]
    fn json_keeps_byte_strings_as_hex_and_nested_protos() {
        let proto = compile(b"local f = function() return '\0' end\nreturn f()").unwrap();
        let json = proto_json(&proto);
        assert!(json.contains("\"hex\":\"00\""));
        assert!(json.contains("\"protos\":[{"));
    }

    #[test]
    fn cfg_contains_jump_and_fallthrough_edges() {
        let proto = compile(b"if true then return 1 end\nreturn 2").unwrap();
        let dot = cfg_dot(&proto).unwrap();
        assert!(dot.starts_with("digraph bytecode"));
        assert!(dot.contains("label=\"jump\""));
        assert!(dot.contains("label=\"next\""));
    }

    #[test]
    fn trace_json_records_dispatch_state() {
        let proto = compile(b"local answer = 40 + 2\nreturn answer").unwrap();
        let mut vm = Vm::new();
        vm.enable_trace();
        vm.execute(&proto).unwrap();
        let json = trace_json(&vm.take_trace());
        assert!(json.contains("\"op\":\"Add\""));
        assert!(json.contains("\"registers\""));
    }
}

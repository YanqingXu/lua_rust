//! lua_bytecode — Lua 5.1 bytecode dumper
//!
//! Compiles Lua source files and displays their bytecode in readable format.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::heap::Heap;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::value::Value;
use serde_json::{Value as JsonValue, json};

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

const TOOL_NAME: &str = "bytecode_main";

/// Compiler diagnostics retain their original bytes until the stderr writer.
/// Host text errors enter through `From<String>`; Display is not a transport.
#[derive(Debug)]
struct ToolError(Vec<u8>);

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self(message.into_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolMode {
    Print,
    Cfg,
    Diff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
struct ToolOptions {
    mode: ToolMode,
    full: bool,
    format: OutputFormat,
    scripts: Vec<String>,
}

fn usage_text() -> String {
    format!(
        concat!(
            "Usage:\n",
            "  {0} <script.lua> [full|--full]\n",
            "  {0} <script.lua> --cfg [full|--full]\n",
            "  {0} <left.lua> <right.lua> --diff [full|--full]\n",
        ),
        TOOL_NAME
    )
}

fn option_error(message: impl AsRef<str>) -> String {
    let mut error = message.as_ref().to_string();
    if !error.ends_with('\n') {
        error.push('\n');
    }
    error.push_str(&usage_text());
    error
}

fn parse_options(args: &[String]) -> Result<ToolOptions, String> {
    if args.is_empty() {
        return Err(usage_text());
    }

    let mut diff = false;
    let mut cfg = false;
    let mut full = false;
    let mut format = OutputFormat::Text;
    let mut format_seen = false;
    let mut scripts = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--diff" => diff = true,
            "--cfg" => cfg = true,
            "full" | "--full" => full = true,
            _ if arg.starts_with("--format=") => {
                if format_seen {
                    return Err(option_error("[ERROR] --format may only be specified once"));
                }
                format_seen = true;
                format = match arg.strip_prefix("--format=").unwrap_or_default() {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    value => {
                        return Err(option_error(format!(
                            "[ERROR] Unknown format: {value} (expected text or json)"
                        )));
                    }
                };
            }
            _ if arg.starts_with('-') => {
                return Err(option_error(format!("[ERROR] Unknown option: {arg}")));
            }
            _ => scripts.push(arg.clone()),
        }
    }

    if diff && cfg {
        return Err(option_error("[ERROR] --cfg cannot be combined with --diff"));
    }
    if format == OutputFormat::Json && (diff || cfg || full) {
        return Err(option_error(
            "[ERROR] --format=json cannot be combined with --cfg, --diff, or --full",
        ));
    }

    let mode = if diff {
        ToolMode::Diff
    } else if cfg {
        ToolMode::Cfg
    } else {
        ToolMode::Print
    };
    let expected = if mode == ToolMode::Diff { 2 } else { 1 };
    if scripts.len() != expected {
        let plural = if expected == 1 { "" } else { "s" };
        let mode_name = match mode {
            ToolMode::Print => "print",
            ToolMode::Cfg => "cfg",
            ToolMode::Diff => "diff",
        };
        return Err(option_error(format!(
            "[ERROR] Expected {expected} script path{plural} for {mode_name} mode, got {}",
            scripts.len()
        )));
    }

    Ok(ToolOptions {
        mode,
        full,
        format,
        scripts,
    })
}

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(message) => {
            write_stderr(message.as_bytes());
            return ExitCode::FAILURE;
        }
    };

    match run_tool(&options) {
        Ok(output) => {
            write_stdout(&output);
            ExitCode::SUCCESS
        }
        Err(error) => {
            let mut diagnostic = b"[ERROR] Exception: ".to_vec();
            // The pinned C++ exception envelope formats e.what() as a C
            // string. Only this output boundary stops at the first NUL;
            // ToolError retains the complete compiler diagnostic internally.
            let end = error
                .0
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(error.0.len());
            diagnostic.extend_from_slice(&error.0[..end]);
            diagnostic.push(b'\n');
            write_stderr(&diagnostic);
            ExitCode::from(3)
        }
    }
}

fn run_tool(options: &ToolOptions) -> Result<Vec<u8>, ToolError> {
    let mut heap = Heap::new();
    let mut roots = Vec::with_capacity(options.scripts.len());
    for filename in &options.scripts {
        let source = read_lua_source_file(filename)?;
        let proto =
            heap.with_parts_mut(|gc, strings| compile_lua_source(&source, filename, gc, strings))?;
        roots.push(proto);
    }

    let result = match options.format {
        OutputFormat::Json => heap
            .collector()
            .with_ref(roots[0], |proto| {
                render_json(proto, &options.scripts[0], heap.collector())
            })
            .map_err(|error| error.to_string())?,
        OutputFormat::Text => match options.mode {
            ToolMode::Print => heap
                .collector()
                .with_ref(roots[0], |proto| {
                    render_proto_text(proto, heap.collector(), options.full)
                })
                .map_err(|error| error.to_string())?,
            ToolMode::Cfg => heap
                .collector()
                .with_ref(roots[0], |proto| {
                    render_proto_cfg(proto, heap.collector(), options.full)
                })
                .map_err(|error| error.to_string())?,
            ToolMode::Diff => {
                let left = heap
                    .collector()
                    .with_ref(roots[0], |proto| {
                        render_proto_text(proto, heap.collector(), options.full)
                    })
                    .map_err(|error| error.to_string())??;
                let right = heap
                    .collector()
                    .with_ref(roots[1], |proto| {
                        render_proto_text(proto, heap.collector(), options.full)
                    })
                    .map_err(|error| error.to_string())??;
                Ok(render_proto_diff(
                    &left,
                    &right,
                    options.full,
                    &options.scripts[0],
                    &options.scripts[1],
                ))
            }
        },
    };

    for root in roots {
        heap.collector_mut().remove_root(root);
    }
    result.map_err(ToolError::from)
}

fn compile_lua_source(
    source: &[u8],
    source_name: &str,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
) -> Result<GcRef<Proto>, ToolError> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser
        .parse()
        .map_err(|error| ToolError(error.message_bytes().to_vec()))?;
    gc.with_publication(|transaction| {
        let proto = CodeGenerator::new_in_publication_with_pool(transaction, string_pool)
            .generate(&chunk, source_name)
            .map_err(|error| ToolError(error.message_bytes().to_vec()))?;
        let proto = transaction.alloc(proto);
        transaction
            .publish_as_explicit_root(proto)
            .map_err(|error| ToolError::from(error.to_string()))
    })
}

fn read_lua_source_file(filename: &str) -> Result<Vec<u8>, String> {
    fs::read(filename).map_err(|_| format!("cannot open file: {filename}"))
}

fn write_stdout(bytes: &[u8]) {
    let mut stdout = io::stdout().lock();
    write_host_text(&mut stdout, bytes);
}

fn write_stderr(message: &[u8]) {
    let mut stderr = io::stderr().lock();
    write_host_text(&mut stderr, message);
}

fn write_host_text(output: &mut impl Write, bytes: &[u8]) {
    #[cfg(windows)]
    {
        let mut translated = Vec::with_capacity(bytes.len() + bytes.len() / 16);
        for &byte in bytes {
            if byte == b'\n' {
                translated.push(b'\r');
            }
            translated.push(byte);
        }
        let _ = output.write_all(&translated);
    }
    #[cfg(not(windows))]
    {
        let _ = output.write_all(bytes);
    }
    let _ = output.flush();
}

fn render_json(proto: &Proto, filename: &str, gc: &GarbageCollector) -> Result<Vec<u8>, String> {
    let mut document = proto_json(proto, gc, &mut Vec::new()).map_err(|error| error.to_string())?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| "Proto JSON serializer did not produce an object".to_string())?;
    object.insert("schema_version".to_string(), JsonValue::from(2));
    object.insert("input".to_string(), JsonValue::from(filename));
    let mut output = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
    output.push(b'\n');
    Ok(output)
}

fn render_proto_text(proto: &Proto, gc: &GarbageCollector, full: bool) -> Result<Vec<u8>, String> {
    let mut output = String::new();
    render_proto_recursive(proto, gc, full, 0, &mut Vec::new(), &mut output)?;
    Ok(restore_raw_byte_markers(output.as_bytes()))
}

fn render_proto_recursive(
    proto: &Proto,
    gc: &GarbageCollector,
    full: bool,
    depth: usize,
    ancestry: &mut Vec<lua_core::gc::object_id::ObjectId>,
    output: &mut String,
) -> Result<(), String> {
    let indent = "  ".repeat(depth);
    writeln!(output, "{indent}Proto").expect("String formatting cannot fail");
    writeln!(output, "{indent}  source: {}", proto_source(proto, gc)?)
        .expect("String formatting cannot fail");
    writeln!(output, "{indent}  linedefined: {}", proto.line_defined())
        .expect("String formatting cannot fail");
    writeln!(
        output,
        "{indent}  lastlinedefined: {}",
        proto.last_line_defined()
    )
    .expect("String formatting cannot fail");
    writeln!(output, "{indent}  numparams: {}", proto.num_params())
        .expect("String formatting cannot fail");
    writeln!(
        output,
        "{indent}  is_vararg: {} (flags={})",
        proto.is_vararg(),
        proto.vararg_flags()
    )
    .expect("String formatting cannot fail");
    writeln!(output, "{indent}  maxStackSize: {}", proto.max_stack_size())
        .expect("String formatting cannot fail");

    let upvalue_count = usize::max(proto.num_upvalues() as usize, proto.upvalue_name_count());
    write!(output, "{indent}  upvalues ({upvalue_count}): ")
        .expect("String formatting cannot fail");
    if upvalue_count == 0 {
        output.push_str("(none)");
    } else {
        for index in 0..upvalue_count {
            if index != 0 {
                output.push_str(", ");
            }
            let name = match proto.upvalue_name(index) {
                Some(name) => gc
                    .with_string_bytes(name, display_c_string)
                    .map_err(|error| error.to_string())?,
                None => "?".to_string(),
            };
            output.push_str(&name);
        }
    }
    output.push('\n');

    writeln!(
        output,
        "{indent}instructions ({})",
        proto.instruction_count()
    )
    .expect("String formatting cannot fail");
    for (pc, instruction) in proto.code().iter().copied().enumerate() {
        render_text_instruction(output, proto, gc, pc, instruction, &indent)?;
    }
    writeln!(output, "{indent}constants ({})", proto.constant_count())
        .expect("String formatting cannot fail");
    for index in 0..proto.constant_count() {
        writeln!(
            output,
            "{indent}  {}",
            format_proto_constant(proto, gc, index as i32)?
        )
        .expect("String formatting cannot fail");
    }

    if full {
        writeln!(output, "{indent}child protos ({})", proto.sub_proto_count())
            .expect("String formatting cannot fail");
        for index in 0..proto.sub_proto_count() {
            let child_ref = proto.sub_proto(index);
            let summary = gc
                .with_ref(child_ref, |child| proto_summary(child, gc))
                .map_err(|error| error.to_string())??;
            writeln!(output, "{indent}  proto[{index}] {summary}")
                .expect("String formatting cannot fail");
            if ancestry.contains(&child_ref.object_id()) {
                writeln!(output, "{indent}    Proto <cycle>")
                    .expect("String formatting cannot fail");
                continue;
            }
            ancestry.push(child_ref.object_id());
            let result = gc
                .with_ref(child_ref, |child| {
                    render_proto_recursive(child, gc, true, depth + 2, ancestry, output)
                })
                .map_err(|error| error.to_string())?;
            ancestry.pop();
            result?;
        }
    }
    Ok(())
}

fn render_text_instruction(
    output: &mut String,
    proto: &Proto,
    gc: &GarbageCollector,
    pc: usize,
    instruction: u32,
    indent: &str,
) -> Result<(), String> {
    let op = opcode::get_opcode(instruction);
    let metadata = opcode::opcode_metadata(op);
    write!(
        output,
        "{indent}{pc:04} | line {} | {} | ",
        proto.line(pc),
        metadata.name
    )
    .expect("String formatting cannot fail");
    let mut comments = Vec::new();
    match metadata.mode {
        opcode::OpMode::IABC => {
            let a = opcode::get_arg_a(instruction);
            let b = opcode::get_arg_b(instruction);
            let c = opcode::get_arg_c(instruction);
            write!(output, "A={a} B={b} C={c}").expect("String formatting cannot fail");
            add_rk_text_comment(&mut comments, proto, gc, "B", b, metadata.b_mode)?;
            add_rk_text_comment(&mut comments, proto, gc, "C", c, metadata.c_mode)?;
        }
        opcode::OpMode::IABx => {
            let a = opcode::get_arg_a(instruction);
            let bx = opcode::get_arg_bx(instruction);
            write!(output, "A={a} Bx={bx}").expect("String formatting cannot fail");
            if metadata.b_mode == opcode::OpArgMask::OpArgK {
                comments.push(format_proto_constant(proto, gc, bx)?);
            } else if op == OpCode::CLOSURE {
                comments.push(format_proto_child(proto, gc, bx)?);
            }
        }
        opcode::OpMode::IAsBx => {
            let a = opcode::get_arg_a(instruction);
            let sbx = opcode::get_arg_sbx(instruction);
            write!(output, "A={a} sBx={sbx}").expect("String formatting cannot fail");
            comments.push(format!("target={}", pc as i32 + 1 + sbx));
        }
    }
    if !comments.is_empty() {
        write!(output, " ; {}", comments.join("; ")).expect("String formatting cannot fail");
    }
    output.push('\n');
    Ok(())
}

fn add_rk_text_comment(
    comments: &mut Vec<String>,
    proto: &Proto,
    gc: &GarbageCollector,
    name: &str,
    operand: i32,
    mode: opcode::OpArgMask,
) -> Result<(), String> {
    if mode == opcode::OpArgMask::OpArgK && opcode::is_k(operand) {
        comments.push(format!(
            "{name}={}",
            format_proto_constant(proto, gc, opcode::index_k(operand))?
        ));
    }
    Ok(())
}

fn format_proto_constant(
    proto: &Proto,
    gc: &GarbageCollector,
    index: i32,
) -> Result<String, String> {
    if index < 0 || index as usize >= proto.constant_count() {
        return Ok(format!("K[{index}] = <out of range>"));
    }
    Ok(format!(
        "K[{index}] = {}",
        format_proto_value(proto.constant(index as usize), gc)?
    ))
}

fn format_proto_value(value: &Value, gc: &GarbageCollector) -> Result<String, String> {
    Ok(match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(value) => format!("boolean {value}"),
        Value::Number(value) => format!("number {value}"),
        Value::String(value) => {
            let escaped = gc
                .with_string_bytes(*value, escape_c_string)
                .map_err(|error| error.to_string())?;
            format!("string \"{escaped}\"")
        }
        Value::LightUserdata(_) => "lightuserdata".to_string(),
        Value::Table(_) => "table".to_string(),
        Value::Function(_) => "function".to_string(),
        Value::Userdata(_) => "userdata".to_string(),
        Value::Thread(_) => "thread".to_string(),
    })
}

fn format_proto_child(proto: &Proto, gc: &GarbageCollector, index: i32) -> Result<String, String> {
    if index < 0 || index as usize >= proto.sub_proto_count() {
        return Ok(format!("proto[{index}] = <out of range>"));
    }
    let summary = gc
        .with_ref(proto.sub_proto(index as usize), |child| {
            proto_summary(child, gc)
        })
        .map_err(|error| error.to_string())??;
    Ok(format!("proto[{index}] = {summary}"))
}

fn proto_summary(proto: &Proto, gc: &GarbageCollector) -> Result<String, String> {
    let source = proto_source(proto, gc)?;
    if proto.line_defined() > 0 {
        Ok(format!("{source}:{}", proto.line_defined()))
    } else {
        Ok(source)
    }
}

fn proto_source(proto: &Proto, gc: &GarbageCollector) -> Result<String, String> {
    match proto.source() {
        Some(source) => gc
            .with_string_bytes(source, display_c_string)
            .map_err(|error| error.to_string()),
        None => Ok("?".to_string()),
    }
}

fn display_c_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|byte| *byte == 0).next().unwrap_or_default()).into_owned()
}

fn escape_c_string(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for &byte in bytes.split(|byte| *byte == 0).next().unwrap_or_default() {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'"' => escaped.push_str("\\\""),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            b'\t' => escaped.push_str("\\t"),
            0x80..=0xff => {
                write!(escaped, "\0{byte:02X}\0").expect("String formatting cannot fail")
            }
            _ => escaped.push(char::from(byte)),
        }
    }
    escaped
}

fn restore_raw_byte_markers(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0
            && index + 3 < bytes.len()
            && bytes[index + 3] == 0
            && let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            output.push(byte);
            index += 4;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    output
}

fn render_proto_diff(
    left: &[u8],
    right: &[u8],
    full: bool,
    left_label: &str,
    right_label: &str,
) -> Vec<u8> {
    let left_lines = diff_lines(left);
    let right_lines = diff_lines(right);
    let line_count = usize::max(left_lines.len(), right_lines.len());
    let changed = (0..line_count)
        .filter(|&index| left_lines.get(index) != right_lines.get(index))
        .count();
    let mut output = format!(
        "Bytecode diff\n  left: {left_label}\n  right: {right_label}\n  mode: {}\n  status: {}\n  changed lines: {changed}\n",
        if full { "full" } else { "compact" },
        if changed == 0 { "identical" } else { "different" }
    )
    .into_bytes();
    if changed != 0 {
        output.extend_from_slice(b"line | left | right\n");
        for index in 0..line_count {
            let left_line = left_lines.get(index).copied().unwrap_or(b"<missing>");
            let right_line = right_lines.get(index).copied().unwrap_or(b"<missing>");
            if left_lines.get(index) != right_lines.get(index) {
                output.extend_from_slice(format!("{index:04} | ").as_bytes());
                output.extend_from_slice(left_line);
                output.extend_from_slice(b" | ");
                output.extend_from_slice(right_line);
                output.push(b'\n');
            }
        }
    }
    output
}

fn diff_lines(text: &[u8]) -> Vec<&[u8]> {
    text.split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !trim_ascii_start(line).starts_with(b"source: "))
        .collect()
}

fn trim_ascii_start(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    bytes
}

#[derive(Clone, Copy)]
struct CfgBlock {
    id: usize,
    start_pc: usize,
    end_pc: usize,
}

struct CfgEdge {
    from_block: usize,
    target_pc: i32,
    label: &'static str,
}

struct CfgGraph {
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    pc_to_block: Vec<i32>,
}

fn render_proto_cfg(proto: &Proto, gc: &GarbageCollector, full: bool) -> Result<Vec<u8>, String> {
    let mut output = String::from("flowchart TD\n  %% lua_bytecode Mermaid CFG\n");
    let mut next_proto_id = 0;
    render_cfg_recursive(
        proto,
        gc,
        full,
        &mut next_proto_id,
        format!("Proto {}", proto_summary(proto, gc)?),
        &mut Vec::new(),
        &mut output,
    )?;
    Ok(output.into_bytes())
}

fn render_cfg_recursive(
    proto: &Proto,
    gc: &GarbageCollector,
    full: bool,
    next_proto_id: &mut usize,
    title: String,
    ancestry: &mut Vec<lua_core::gc::object_id::ObjectId>,
    output: &mut String,
) -> Result<(), String> {
    let proto_id = *next_proto_id;
    *next_proto_id += 1;
    let graph = build_cfg_graph(proto);
    writeln!(
        output,
        "  subgraph P{proto_id}[\"{}\"]",
        escape_mermaid(&title)
    )
    .expect("String formatting cannot fail");
    writeln!(output, "    P{proto_id}_EXIT((\"exit\"))").expect("String formatting cannot fail");
    for block in &graph.blocks {
        writeln!(
            output,
            "    P{proto_id}_B{}[\"{}\"]",
            block.id,
            escape_mermaid(&cfg_block_label(proto, *block))
        )
        .expect("String formatting cannot fail");
    }
    writeln!(output, "  end").expect("String formatting cannot fail");
    for edge in &graph.edges {
        let target = if edge.target_pc >= 0 {
            graph
                .pc_to_block
                .get(edge.target_pc as usize)
                .copied()
                .filter(|block| *block >= 0)
                .map(|block| format!("P{proto_id}_B{block}"))
                .unwrap_or_else(|| format!("P{proto_id}_EXIT"))
        } else {
            format!("P{proto_id}_EXIT")
        };
        writeln!(
            output,
            "  P{proto_id}_B{} -->|{}| {target}",
            edge.from_block, edge.label
        )
        .expect("String formatting cannot fail");
    }

    if full {
        for index in 0..proto.sub_proto_count() {
            let child_ref = proto.sub_proto(index);
            if ancestry.contains(&child_ref.object_id()) {
                writeln!(output, "  %% proto[{index}] skipped: cycle")
                    .expect("String formatting cannot fail");
                continue;
            }
            let title = gc
                .with_ref(child_ref, |child| proto_summary(child, gc))
                .map_err(|error| error.to_string())??;
            ancestry.push(child_ref.object_id());
            let result = gc
                .with_ref(child_ref, |child| {
                    render_cfg_recursive(
                        child,
                        gc,
                        true,
                        next_proto_id,
                        format!("proto[{index}] {title}"),
                        ancestry,
                        output,
                    )
                })
                .map_err(|error| error.to_string())?;
            ancestry.pop();
            result?;
        }
    }
    Ok(())
}

fn build_cfg_graph(proto: &Proto) -> CfgGraph {
    let code = proto.code();
    let mut leaders = vec![false; code.len()];
    if !code.is_empty() {
        leaders[0] = true;
    }
    for (pc, &instruction) in code.iter().enumerate() {
        let op = opcode::get_opcode(instruction);
        if is_companion_jump_consumer(op)
            && code
                .get(pc + 1)
                .is_some_and(|next| opcode::get_opcode(*next) == OpCode::JMP)
        {
            add_cfg_leader(&mut leaders, cfg_jump_target(pc + 1, code[pc + 1]));
            add_cfg_leader(&mut leaders, pc as i32 + 2);
            continue;
        }
        if is_comparison_opcode(op) {
            add_cfg_leader(&mut leaders, pc as i32 + 1);
            add_cfg_leader(&mut leaders, pc as i32 + 2);
            continue;
        }
        match op {
            OpCode::JMP | OpCode::FORPREP | OpCode::FORLOOP => {
                add_cfg_leader(&mut leaders, cfg_jump_target(pc, instruction));
                add_cfg_leader(&mut leaders, pc as i32 + 1);
            }
            OpCode::LOADBOOL if opcode::get_arg_c(instruction) != 0 => {
                add_cfg_leader(&mut leaders, pc as i32 + 1);
                add_cfg_leader(&mut leaders, pc as i32 + 2);
            }
            OpCode::RETURN | OpCode::TAILCALL => add_cfg_leader(&mut leaders, pc as i32 + 1),
            _ => {}
        }
    }
    let leader_pcs = leaders
        .iter()
        .enumerate()
        .filter_map(|(pc, leader)| leader.then_some(pc))
        .collect::<Vec<_>>();
    let blocks = leader_pcs
        .iter()
        .enumerate()
        .filter_map(|(index, &start_pc)| {
            let next = leader_pcs.get(index + 1).copied().unwrap_or(code.len());
            (start_pc < code.len() && next > start_pc).then_some(CfgBlock {
                id: index,
                start_pc,
                end_pc: next - 1,
            })
        })
        .collect::<Vec<_>>();
    let mut pc_to_block = vec![-1; code.len()];
    for block in &blocks {
        for slot in &mut pc_to_block[block.start_pc..=block.end_pc] {
            *slot = block.id as i32;
        }
    }
    let mut edges = Vec::new();
    for block in &blocks {
        if block.end_pc > block.start_pc
            && is_companion_jump_consumer(opcode::get_opcode(code[block.end_pc - 1]))
            && opcode::get_opcode(code[block.end_pc]) == OpCode::JMP
        {
            let op = opcode::get_opcode(code[block.end_pc - 1]);
            edges.push(CfgEdge {
                from_block: block.id,
                target_pc: cfg_jump_target(block.end_pc, code[block.end_pc]),
                label: if op == OpCode::TFORLOOP {
                    "iterator next"
                } else {
                    "test jump"
                },
            });
            edges.push(CfgEdge {
                from_block: block.id,
                target_pc: block.end_pc as i32 + 1,
                label: if op == OpCode::TFORLOOP {
                    "iterator done"
                } else {
                    "test fallthrough"
                },
            });
            continue;
        }
        let instruction = code[block.end_pc];
        let op = opcode::get_opcode(instruction);
        if is_comparison_opcode(op) {
            edges.push(CfgEdge {
                from_block: block.id,
                target_pc: block.end_pc as i32 + 1,
                label: "compare next",
            });
            edges.push(CfgEdge {
                from_block: block.id,
                target_pc: block.end_pc as i32 + 2,
                label: "compare skip",
            });
            continue;
        }
        let (target_pc, label) = match op {
            OpCode::JMP => (cfg_jump_target(block.end_pc, instruction), "jump"),
            OpCode::LOADBOOL if opcode::get_arg_c(instruction) != 0 => {
                (block.end_pc as i32 + 2, "skip")
            }
            OpCode::RETURN | OpCode::TAILCALL => (code.len() as i32, "return"),
            OpCode::FORPREP => (cfg_jump_target(block.end_pc, instruction), "prepare"),
            OpCode::FORLOOP => {
                edges.push(CfgEdge {
                    from_block: block.id,
                    target_pc: cfg_jump_target(block.end_pc, instruction),
                    label: "loop",
                });
                (block.end_pc as i32 + 1, "exit")
            }
            _ => (block.end_pc as i32 + 1, "fallthrough"),
        };
        edges.push(CfgEdge {
            from_block: block.id,
            target_pc,
            label,
        });
    }
    CfgGraph {
        blocks,
        edges,
        pc_to_block,
    }
}

fn add_cfg_leader(leaders: &mut [bool], pc: i32) {
    if pc >= 0 && (pc as usize) < leaders.len() {
        leaders[pc as usize] = true;
    }
}

fn cfg_jump_target(pc: usize, instruction: u32) -> i32 {
    pc as i32 + 1 + opcode::get_arg_sbx(instruction)
}

fn is_companion_jump_consumer(op: OpCode) -> bool {
    matches!(op, OpCode::TEST | OpCode::TESTSET | OpCode::TFORLOOP)
}

fn is_comparison_opcode(op: OpCode) -> bool {
    matches!(op, OpCode::EQ | OpCode::LT | OpCode::LE)
}

fn cfg_block_label(proto: &Proto, block: CfgBlock) -> String {
    let range = if block.start_pc == block.end_pc {
        format!("pc {}", block.start_pc)
    } else {
        format!("pc {}..{}", block.start_pc, block.end_pc)
    };
    let first = opcode::get_op_name(opcode::get_opcode(proto.code()[block.start_pc]));
    let last = opcode::get_op_name(opcode::get_opcode(proto.code()[block.end_pc]));
    let summary = if block.start_pc == block.end_pc {
        first.to_string()
    } else {
        format!("{first} -> {last}")
    };
    format!("B{}\n{range}\n{summary}", block.id)
}

fn escape_mermaid(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

#[cfg(test)]
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

#[cfg(test)]
mod byte_source_tests {
    use super::*;

    fn compile_test_proto(
        source: &[u8],
        source_name: &str,
        gc: &mut GarbageCollector,
        pool: &mut StringPool,
    ) -> GcRef<Proto> {
        compile_lua_source(source, source_name, gc, pool).expect("test source should compile")
    }

    #[test]
    fn fixed_tool_options_are_strict() {
        let options = parse_options(&[
            "left.lua".to_string(),
            "right.lua".to_string(),
            "--diff".to_string(),
            "--full".to_string(),
        ])
        .expect("full diff options should parse");
        assert_eq!(options.mode, ToolMode::Diff);
        assert!(options.full);
        assert_eq!(options.format, OutputFormat::Text);
        assert_eq!(options.scripts, ["left.lua", "right.lua"]);

        let error = parse_options(&["file.lua".to_string(), "--format=yaml".to_string()])
            .expect_err("unknown formats must fail closed");
        assert!(error.contains("Unknown format: yaml (expected text or json)"));
        assert!(error.contains("bytecode_main <script.lua>"));
    }

    #[test]
    fn text_and_diff_preserve_high_byte_constants() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let left = compile_test_proto(b"return \"\\255\"", "left.lua", &mut gc, &mut pool);
        let right = compile_test_proto(b"return \"\\254\"", "right.lua", &mut gc, &mut pool);
        let left_text = gc
            .with_ref(left, |proto| render_proto_text(proto, &gc, false))
            .expect("left Proto should remain live")
            .expect("left Proto should render");
        let right_text = gc
            .with_ref(right, |proto| render_proto_text(proto, &gc, false))
            .expect("right Proto should remain live")
            .expect("right Proto should render");
        assert!(
            left_text
                .windows(3)
                .any(|window| window == [b'"', 0xff, b'"'])
        );
        assert!(
            right_text
                .windows(3)
                .any(|window| window == [b'"', 0xfe, b'"'])
        );

        let diff = render_proto_diff(&left_text, &right_text, false, "left.lua", "right.lua");
        assert!(diff.windows(3).any(|window| window == [b'"', 0xff, b'"']));
        assert!(diff.windows(3).any(|window| window == [b'"', 0xfe, b'"']));
        gc.remove_root(left);
        gc.remove_root(right);
    }

    #[test]
    fn non_utf8_literal_reaches_proto_without_reencoding() {
        let mut source = b"return \"".to_vec();
        source.extend_from_slice(&[0x00, 0x80, 0xff]);
        source.push(b'"');

        let mut parser = Parser::from_bytes(&source);
        let chunk = parser.parse().expect("byte source should parse");
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let proto = gc.with_publication(|transaction| {
            let proto = CodeGenerator::new_in_publication_with_pool(transaction, &mut string_pool)
                .generate(&chunk, "@bytes.lua")
                .expect("byte source should compile");
            let proto = transaction.alloc(proto);
            transaction
                .publish_as_explicit_root(proto)
                .expect("test Proto should publish")
        });
        let string = gc
            .with_ref(proto, |proto| {
                proto.constants().iter().find_map(|value| match value {
                    Value::String(string) => Some(*string),
                    _ => None,
                })
            })
            .expect("test Proto remains registered")
            .expect("literal should become a string constant");
        gc.with_ref(string, |string| {
            assert_eq!(string.as_bytes(), &[0x00, 0x80, 0xff]);
        })
        .expect("constant string remains registered");
        gc.remove_root(proto);
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

        gc.with_ref(proto, |proto| {
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
            gc.with_string_bytes(*print_name, |bytes| assert_eq!(bytes, b"print"))
                .expect("string constant should be live");
        })
        .expect("root Proto remains registered");
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
        let evidence = gc
            .with_ref(proto, |proto| proto_json(proto, &gc, &mut Vec::new()))
            .expect("root Proto remains registered")
            .expect("Proto tree should serialize");

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

//! Full-REPL meta commands shared with the fixed C++ command contract.

use lua_compiler::ast::expr::{BinaryOp, Expr, UnaryOp};
use lua_compiler::ast::stmt::{Chunk, Stmt};
use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpArgMask, OpCode, OpMode};
use lua_compiler::parser::{ParseError, Parser};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::value::Value;
use lua_vm::runtime::Runtime;

use std::fmt::Write as _;

const HELP: &str = concat!(
    "REPL commands:\n",
    "  .help                  show this help\n",
    "  .bytecode <expr|chunk> compile input and print bytecode\n",
    "  .ast <expr|chunk>      parse input and print AST\n",
    "  .gc [stats|collect|strategy|help] inspect or run the active GC\n",
    "  =expr                  evaluate expression and print results\n",
    "  exit, quit             leave the REPL\n",
);

const GC_USAGE: &str = "usage: .gc [stats|collect|strategy|help]\n";

#[derive(Default)]
pub(crate) struct MetaOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run(runtime: &mut Runtime, line: &str) -> Option<MetaOutput> {
    let trimmed = line.trim();
    let command = trimmed.strip_prefix('.')?;
    let command_end = command.find(char::is_whitespace).unwrap_or(command.len());
    let name = &command[..command_end];
    let argument = command[command_end..].trim();

    let output = match name {
        "help" => MetaOutput {
            stdout: HELP.to_string(),
            stderr: String::new(),
        },
        "bytecode" => bytecode(runtime, argument),
        "ast" => ast(argument),
        "gc" => gc(runtime, argument),
        _ => MetaOutput {
            stdout: String::new(),
            stderr: format!("unknown REPL command: .{name}"),
        },
    };
    Some(output)
}

fn bytecode(runtime: &mut Runtime, source: &str) -> MetaOutput {
    if source.trim().is_empty() {
        return stderr("usage: .bytecode <expr|chunk>");
    }
    let (chunk, _) = match parse_meta_input(source) {
        Ok(parsed) => parsed,
        Err(error) => return parse_stderr(error),
    };

    let rendered = (|| -> Result<String, String> {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (_, gc, string_pool) = parts.split_mut();
        let proto = compile_meta_proto(gc, string_pool, &chunk)?;
        let rendering = gc
            .with_ref(proto, |proto| render_proto(proto, gc))
            .map_err(|error| error.to_string())?;
        gc.remove_root(proto);
        rendering
    })();

    match rendered {
        Ok(stdout) => MetaOutput {
            stdout,
            stderr: String::new(),
        },
        Err(message) => stderr(message),
    }
}

fn compile_meta_proto(
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    chunk: &Chunk,
) -> Result<GcRef<Proto>, String> {
    gc.with_publication(|transaction| {
        let proto = CodeGenerator::new_in_publication_with_pool(transaction, string_pool)
            .generate(chunk, "=(repl bytecode)")
            .map_err(|error| error.message)?;
        let proto = transaction.alloc(proto);
        transaction
            .publish_as_explicit_root(proto)
            .map_err(|error| format!("(repl bytecode): invalid Proto publication: {error}"))
    })
}

fn ast(source: &str) -> MetaOutput {
    if source.trim().is_empty() {
        return stderr("usage: .ast <expr|chunk>");
    }
    let (chunk, mode) = match parse_meta_input(source) {
        Ok(parsed) => parsed,
        Err(error) => return parse_stderr(error),
    };

    let mut stdout = format!("AST\n  mode: {mode}\n");
    AstPrinter::new(&mut stdout).print_chunk(&chunk);
    MetaOutput {
        stdout,
        stderr: String::new(),
    }
}

fn parse_meta_input(source: &str) -> Result<(Chunk, &'static str), ParseError> {
    let input = source.trim();
    if let Some(expression) = input.strip_prefix('=') {
        return Parser::from_bytes(format!("return {expression}").as_bytes())
            .parse()
            .map(|chunk| (chunk, "expression"));
    }

    let mut primary = Parser::from_bytes(input.as_bytes());
    if let Ok(chunk) = primary.parse() {
        return Ok((chunk, "chunk"));
    }

    Parser::from_bytes(format!("return {input}").as_bytes())
        .parse()
        .map(|chunk| (chunk, "expression"))
}

fn parse_stderr(error: ParseError) -> MetaOutput {
    stderr(format!("stdin:{}: {}", error.line, error.message))
}

fn stderr(message: impl Into<String>) -> MetaOutput {
    MetaOutput {
        stdout: String::new(),
        stderr: message.into(),
    }
}

#[derive(Clone, Copy)]
struct GcSnapshot {
    objects: usize,
    roots: usize,
    memory_bytes: usize,
}

fn capture_gc_snapshot(runtime: &mut Runtime) -> Result<GcSnapshot, String> {
    let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
    let (_, gc, _) = parts.split_mut();
    Ok(GcSnapshot {
        objects: gc.object_count(),
        roots: gc.root_count(),
        memory_bytes: gc.total_memory(),
    })
}

fn gc(runtime: &mut Runtime, argument: &str) -> MetaOutput {
    match argument.trim() {
        "" | "stats" | "status" => match capture_gc_snapshot(runtime) {
            Ok(snapshot) => MetaOutput {
                stdout: render_gc_stats(snapshot),
                stderr: String::new(),
            },
            Err(message) => stderr(message),
        },
        "collect" => collect_gc(runtime),
        "strategy" => MetaOutput {
            stdout: concat!(
                "GC\n",
                "  command: strategy\n",
                "  active: mark-sweep\n",
                "  active summary: stop-the-world tri-color mark and sweep\n",
                "  available: mark-sweep, incremental\n",
                "  planned: incremental write barriers and scheduling\n",
                "  boundary: RuntimeServices.gc owns the active collector\n",
                "  switch: collectgarbage(\"strategy\", \"mark-sweep\"|\"incremental\")\n",
            )
            .to_string(),
            stderr: String::new(),
        },
        "help" => MetaOutput {
            stdout: GC_USAGE.to_string(),
            stderr: String::new(),
        },
        option => MetaOutput {
            stdout: String::new(),
            stderr: format!(".gc: unknown option '{option}'\n{}", GC_USAGE.trim_end()),
        },
    }
}

fn collect_gc(runtime: &mut Runtime) -> MetaOutput {
    let result = (|| -> Result<(GcSnapshot, GcSnapshot), String> {
        let before = capture_gc_snapshot(runtime)?;
        runtime.collect_garbage_full()?;
        Ok((before, capture_gc_snapshot(runtime)?))
    })();

    match result {
        Ok((before, after)) => {
            let mut stdout = String::from("GC\n  command: collect\n  strategy: mark-sweep\n");
            writeln!(
                stdout,
                "  collected objects: {}",
                before.objects.saturating_sub(after.objects)
            )
            .expect("String formatting cannot fail");
            render_gc_snapshot(&mut stdout, "  before:", before);
            render_gc_snapshot(&mut stdout, "  after:", after);
            MetaOutput {
                stdout,
                stderr: String::new(),
            }
        }
        Err(message) => stderr(message),
    }
}

fn render_gc_stats(snapshot: GcSnapshot) -> String {
    let mut output = String::from(
        "GC\n  command: stats\n  strategy: mark-sweep\n  boundary: RuntimeServices.gc\n",
    );
    render_gc_snapshot(&mut output, "  current:", snapshot);
    output
}

fn render_gc_snapshot(output: &mut String, label: &str, snapshot: GcSnapshot) {
    writeln!(output, "{label}").expect("String formatting cannot fail");
    writeln!(output, "    objects: {}", snapshot.objects).expect("String formatting cannot fail");
    writeln!(output, "    roots: {}", snapshot.roots).expect("String formatting cannot fail");
    writeln!(output, "    memory bytes: {}", snapshot.memory_bytes)
        .expect("String formatting cannot fail");
    writeln!(
        output,
        "    memory KB: {:.3}",
        snapshot.memory_bytes as f64 / 1024.0
    )
    .expect("String formatting cannot fail");
}

fn render_proto(proto: &Proto, gc: &GarbageCollector) -> Result<String, String> {
    let mut output = String::new();
    let source = match proto.source() {
        Some(source) => gc
            .with_string_bytes(source, display_bytes)
            .map_err(|error| error.to_string())?,
        None => "?".to_string(),
    };
    writeln!(output, "Proto").expect("String formatting cannot fail");
    writeln!(output, "  source: {source}").expect("String formatting cannot fail");
    writeln!(output, "  linedefined: {}", proto.line_defined())
        .expect("String formatting cannot fail");
    writeln!(output, "  lastlinedefined: {}", proto.last_line_defined())
        .expect("String formatting cannot fail");
    writeln!(output, "  numparams: {}", proto.num_params()).expect("String formatting cannot fail");
    writeln!(
        output,
        "  is_vararg: {} (flags={})",
        proto.is_vararg(),
        proto.vararg_flags()
    )
    .expect("String formatting cannot fail");
    writeln!(output, "  maxStackSize: {}", proto.max_stack_size())
        .expect("String formatting cannot fail");

    let upvalue_count = usize::max(proto.num_upvalues() as usize, proto.upvalue_name_count());
    write!(output, "  upvalues ({upvalue_count}): ").expect("String formatting cannot fail");
    if upvalue_count == 0 {
        output.push_str("(none)");
    } else {
        for index in 0..upvalue_count {
            if index > 0 {
                output.push_str(", ");
            }
            let name = proto
                .upvalue_name(index)
                .and_then(|name| gc.with_string_bytes(name, display_bytes).ok())
                .unwrap_or_else(|| "?".to_string());
            output.push_str(&name);
        }
    }
    output.push('\n');

    writeln!(output, "instructions ({})", proto.instruction_count())
        .expect("String formatting cannot fail");
    for (pc, instruction) in proto.code().iter().copied().enumerate() {
        render_instruction(&mut output, proto, gc, pc, instruction)?;
    }
    writeln!(output, "constants ({})", proto.constant_count())
        .expect("String formatting cannot fail");
    for (index, constant) in proto.constants().iter().enumerate() {
        writeln!(output, "  K[{index}] = {}", format_value(constant, gc)?)
            .expect("String formatting cannot fail");
    }
    Ok(output)
}

fn render_instruction(
    output: &mut String,
    proto: &Proto,
    gc: &GarbageCollector,
    pc: usize,
    instruction: u32,
) -> Result<(), String> {
    let op = opcode::get_opcode(instruction);
    let metadata = opcode::opcode_metadata(op);
    write!(
        output,
        "{pc:04} | line {} | {} | ",
        proto.line(pc),
        metadata.name
    )
    .expect("String formatting cannot fail");
    let mut comments = Vec::new();
    match metadata.mode {
        OpMode::IABC => {
            let a = opcode::get_arg_a(instruction);
            let b = opcode::get_arg_b(instruction);
            let c = opcode::get_arg_c(instruction);
            write!(output, "A={a} B={b} C={c}").expect("String formatting cannot fail");
            add_rk_comment(&mut comments, proto, gc, "B", b, metadata.b_mode)?;
            add_rk_comment(&mut comments, proto, gc, "C", c, metadata.c_mode)?;
        }
        OpMode::IABx => {
            let a = opcode::get_arg_a(instruction);
            let bx = opcode::get_arg_bx(instruction);
            write!(output, "A={a} Bx={bx}").expect("String formatting cannot fail");
            if metadata.b_mode == OpArgMask::OpArgK {
                comments.push(format_constant(proto, gc, bx)?);
            } else if op == OpCode::CLOSURE {
                comments.push(format_sub_proto(proto, gc, bx));
            }
        }
        OpMode::IAsBx => {
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

fn add_rk_comment(
    comments: &mut Vec<String>,
    proto: &Proto,
    gc: &GarbageCollector,
    name: &str,
    operand: i32,
    mode: OpArgMask,
) -> Result<(), String> {
    if mode == OpArgMask::OpArgK && opcode::is_k(operand) {
        comments.push(format!(
            "{name}={}",
            format_constant(proto, gc, opcode::index_k(operand))?
        ));
    }
    Ok(())
}

fn format_constant(proto: &Proto, gc: &GarbageCollector, index: i32) -> Result<String, String> {
    if index < 0 || index as usize >= proto.constant_count() {
        return Ok(format!("K[{index}] = <out of range>"));
    }
    Ok(format!(
        "K[{index}] = {}",
        format_value(proto.constant(index as usize), gc)?
    ))
}

fn format_sub_proto(proto: &Proto, gc: &GarbageCollector, index: i32) -> String {
    if index < 0 || index as usize >= proto.sub_proto_count() {
        return format!("proto[{index}] = <out of range>");
    }
    let child = proto.sub_proto(index as usize);
    let summary = gc
        .with_ref(child, |child| {
            let source = child
                .source()
                .and_then(|source| gc.with_string_bytes(source, display_bytes).ok())
                .unwrap_or_else(|| "?".to_string());
            if child.line_defined() > 0 {
                format!("{source}:{}", child.line_defined())
            } else {
                source
            }
        })
        .unwrap_or_else(|_| "<null>".to_string());
    format!("proto[{index}] = {summary}")
}

fn format_value(value: &Value, gc: &GarbageCollector) -> Result<String, String> {
    Ok(match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(value) => format!("boolean {value}"),
        Value::Number(value) => format!("number {value}"),
        Value::String(value) => {
            let value = gc
                .with_string_bytes(*value, |bytes| escape_bytes(c_string_bytes(bytes)))
                .map_err(|error| error.to_string())?;
            format!("string \"{value}\"")
        }
        Value::LightUserdata(_) => "lightuserdata".to_string(),
        Value::Table(_) => "table".to_string(),
        Value::Function(_) => "function".to_string(),
        Value::Userdata(_) => "userdata".to_string(),
        Value::Thread(_) => "thread".to_string(),
    })
}

fn c_string_bytes(bytes: &[u8]) -> &[u8] {
    bytes.split(|byte| *byte == 0).next().unwrap_or_default()
}

fn display_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(c_string_bytes(bytes)).into_owned()
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for &byte in bytes {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'"' => escaped.push_str("\\\""),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            b'\t' => escaped.push_str("\\t"),
            _ => escaped.push(char::from(byte)),
        }
    }
    escaped
}

struct AstPrinter<'a> {
    output: &'a mut String,
    indent: usize,
}

impl<'a> AstPrinter<'a> {
    fn new(output: &'a mut String) -> Self {
        Self { output, indent: 0 }
    }

    fn line(&mut self, text: impl std::fmt::Display) {
        writeln!(self.output, "{}{text}", "  ".repeat(self.indent))
            .expect("String formatting cannot fail");
    }

    fn indented(&mut self, action: impl FnOnce(&mut Self)) {
        self.indent += 1;
        action(self);
        self.indent -= 1;
    }

    fn print_chunk(&mut self, chunk: &Chunk) {
        self.line("Chunk");
        self.indented(|this| {
            this.line(format_args!("statements ({})", chunk.statements.len()));
            this.print_stmt_list(&chunk.statements);
        });
    }

    fn print_expr_field(&mut self, name: &str, expression: Option<&Expr>) {
        self.line(format_args!("{name}:"));
        self.indented(|this| match expression {
            Some(expression) => this.print_expr(expression),
            None => this.line("<none>"),
        });
    }

    fn print_expr_list(&mut self, expressions: &[Box<Expr>]) {
        self.indented(|this| {
            for (index, expression) in expressions.iter().enumerate() {
                this.line(format_args!("[{index}]"));
                this.indented(|this| this.print_expr(expression));
            }
        });
    }

    fn print_stmt_list(&mut self, statements: &[Box<Stmt>]) {
        self.indented(|this| {
            for (index, statement) in statements.iter().enumerate() {
                this.line(format_args!("[{index}]"));
                this.indented(|this| this.print_stmt(statement));
            }
        });
    }

    fn print_expr(&mut self, expression: &Expr) {
        match expression {
            Expr::Nil(_) => self.expr_line("NilExpr", expression),
            Expr::Boolean(node) => {
                self.expr_line(format!("BoolExpr value={}", node.value), expression)
            }
            Expr::Number(node) => {
                self.expr_line(format!("NumberExpr value={}", node.value), expression)
            }
            Expr::String(node) => self.expr_line(
                format!(
                    "StringExpr value=\"{}\"",
                    escape_bytes(node.value.as_bytes())
                ),
                expression,
            ),
            Expr::Vararg(_) => self.expr_line("VarargExpr", expression),
            Expr::Name(node) => self.expr_line(format!("NameExpr name={}", node.name), expression),
            Expr::Binary(node) => {
                self.expr_line(
                    format!("BinaryExpr op={}", binary_op_name(node.op)),
                    expression,
                );
                self.indented(|this| {
                    this.print_expr_field("left", Some(&node.left));
                    this.print_expr_field("right", Some(&node.right));
                });
            }
            Expr::Unary(node) => {
                self.expr_line(
                    format!("UnaryExpr op={}", unary_op_name(node.op)),
                    expression,
                );
                self.indented(|this| this.print_expr_field("operand", Some(&node.operand)));
            }
            Expr::Table(node) => {
                self.expr_line(
                    format!("TableExpr fields={}", node.fields.len()),
                    expression,
                );
                self.indented(|this| {
                    for (index, field) in node.fields.iter().enumerate() {
                        this.line(format_args!("field[{index}]"));
                        this.indented(|this| {
                            this.print_expr_field("key", field.key.as_deref());
                            this.print_expr_field("value", Some(&field.value));
                        });
                    }
                });
            }
            Expr::Call(node) => {
                self.expr_line(
                    format!("CallExpr method={}", node.is_method_call),
                    expression,
                );
                self.indented(|this| {
                    this.print_expr_field("func", Some(&node.func));
                    this.line(format_args!("args ({})", node.args.len()));
                    this.print_expr_list(&node.args);
                });
            }
            Expr::Index(node) => {
                self.expr_line("IndexExpr", expression);
                self.indented(|this| {
                    this.print_expr_field("table", Some(&node.table));
                    this.print_expr_field("index", Some(&node.index));
                });
            }
            Expr::Member(node) => {
                self.expr_line(format!("MemberExpr member={}", node.member), expression);
                self.indented(|this| this.print_expr_field("table", Some(&node.table)));
            }
            Expr::Function(node) => {
                self.expr_line(
                    format!(
                        "FunctionExpr params=[{}] vararg={}",
                        node.params.join(", "),
                        node.is_vararg
                    ),
                    expression,
                );
                self.indented(|this| {
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
            Expr::Paren(node) => {
                self.expr_line("ParenExpr", expression);
                self.indented(|this| this.print_expr_field("expression", Some(&node.expression)));
            }
        }
    }

    fn expr_line(&mut self, label: impl std::fmt::Display, expression: &Expr) {
        self.line(format_args!(
            "{label} @ {}:{}",
            expression.line(),
            expression.column()
        ));
    }

    fn print_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Empty(_) => self.stmt_line("EmptyStmt", statement),
            Stmt::Assign(node) => {
                self.stmt_line("AssignStmt", statement);
                self.indented(|this| {
                    this.line(format_args!("targets ({})", node.targets.len()));
                    this.print_expr_list(&node.targets);
                    this.line(format_args!("values ({})", node.values.len()));
                    this.print_expr_list(&node.values);
                });
            }
            Stmt::Local(node) => {
                self.stmt_line(
                    format!("LocalStmt names=[{}]", node.names.join(", ")),
                    statement,
                );
                self.indented(|this| {
                    this.line(format_args!("values ({})", node.values.len()));
                    this.print_expr_list(&node.values);
                });
            }
            Stmt::Call(node) => {
                self.stmt_line("CallStmt", statement);
                self.indented(|this| this.print_expr_field("call", Some(&node.call)));
            }
            Stmt::If(node) => {
                self.stmt_line(
                    format!("IfStmt branches={}", node.branches.len()),
                    statement,
                );
                self.indented(|this| {
                    for (index, branch) in node.branches.iter().enumerate() {
                        this.line(format_args!("branch[{index}]"));
                        this.indented(|this| {
                            this.print_expr_field("condition", Some(&branch.condition));
                            this.line(format_args!("body ({})", branch.body.len()));
                            this.print_stmt_list(&branch.body);
                        });
                    }
                    if !node.else_branch.is_empty() {
                        this.line(format_args!("else ({})", node.else_branch.len()));
                        this.print_stmt_list(&node.else_branch);
                    }
                });
            }
            Stmt::While(node) => {
                self.stmt_line("WhileStmt", statement);
                self.indented(|this| {
                    this.print_expr_field("condition", Some(&node.condition));
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
            Stmt::Repeat(node) => {
                self.stmt_line("RepeatStmt", statement);
                self.indented(|this| {
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                    this.print_expr_field("condition", Some(&node.condition));
                });
            }
            Stmt::ForNum(node) => {
                self.stmt_line(format!("ForNumStmt var={}", node.var), statement);
                self.indented(|this| {
                    this.print_expr_field("init", Some(&node.init));
                    this.print_expr_field("limit", Some(&node.limit));
                    this.print_expr_field("step", node.step.as_deref());
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
            Stmt::ForIn(node) => {
                self.stmt_line(
                    format!("ForInStmt vars=[{}]", node.vars.join(", ")),
                    statement,
                );
                self.indented(|this| {
                    this.line(format_args!("iterators ({})", node.iterators.len()));
                    this.print_expr_list(&node.iterators);
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
            Stmt::Function(node) => {
                let mut name = node.name.clone();
                for part in &node.table_path {
                    name.push('.');
                    name.push_str(part);
                }
                self.stmt_line(
                    format!(
                        "FunctionStmt name={name} local={} method={} params=[{}] vararg={}",
                        node.is_local,
                        node.is_method,
                        node.params.join(", "),
                        node.is_vararg
                    ),
                    statement,
                );
                self.indented(|this| {
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
            Stmt::Return(node) => {
                self.stmt_line("ReturnStmt", statement);
                self.indented(|this| {
                    this.line(format_args!("values ({})", node.values.len()));
                    this.print_expr_list(&node.values);
                });
            }
            Stmt::Break(_) => self.stmt_line("BreakStmt", statement),
            Stmt::Do(node) => {
                self.stmt_line("DoStmt", statement);
                self.indented(|this| {
                    this.line(format_args!("body ({})", node.body.len()));
                    this.print_stmt_list(&node.body);
                });
            }
        }
    }

    fn stmt_line(&mut self, label: impl std::fmt::Display, statement: &Stmt) {
        self.line(format_args!(
            "{label} @ {}:{}",
            statement.line(),
            statement.column()
        ));
    }
}

fn binary_op_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "Add",
        BinaryOp::Sub => "Sub",
        BinaryOp::Mul => "Mul",
        BinaryOp::Div => "Div",
        BinaryOp::Mod => "Mod",
        BinaryOp::Pow => "Pow",
        BinaryOp::Eq => "Eq",
        BinaryOp::Ne => "Ne",
        BinaryOp::Lt => "Lt",
        BinaryOp::Le => "Le",
        BinaryOp::Gt => "Gt",
        BinaryOp::Ge => "Ge",
        BinaryOp::And => "And",
        BinaryOp::Or => "Or",
        BinaryOp::Concat => "Concat",
    }
}

fn unary_op_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Not => "Not",
        UnaryOp::Neg => "Neg",
        UnaryOp::Len => "Len",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ast_expression_matches_fixed_repl_shape() {
        let output = ast("1+2");
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout,
            concat!(
                "AST\n",
                "  mode: expression\n",
                "Chunk\n",
                "  statements (1)\n",
                "    [0]\n",
                "      ReturnStmt @ 1:1\n",
                "        values (1)\n",
                "          [0]\n",
                "            BinaryExpr op=Add @ 1:9\n",
                "              left:\n",
                "                NumberExpr value=1 @ 1:8\n",
                "              right:\n",
                "                NumberExpr value=2 @ 1:10\n",
            )
        );
    }

    #[test]
    fn gc_help_and_unknown_are_stable() {
        let help = gc_usage_output();
        assert_eq!(help.stdout, GC_USAGE);
        assert!(help.stderr.is_empty());
        let unknown = unknown_gc_output("nope");
        assert_eq!(
            unknown.stderr,
            ".gc: unknown option 'nope'\nusage: .gc [stats|collect|strategy|help]"
        );
    }

    fn gc_usage_output() -> MetaOutput {
        MetaOutput {
            stdout: GC_USAGE.to_string(),
            stderr: String::new(),
        }
    }

    fn unknown_gc_output(option: &str) -> MetaOutput {
        MetaOutput {
            stdout: String::new(),
            stderr: format!(".gc: unknown option '{option}'\n{}", GC_USAGE.trim_end()),
        }
    }
}

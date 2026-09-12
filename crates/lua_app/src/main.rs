//! lua_app: Lua 5.1 command-line runner.

mod repl_completion;
mod repl_console;
mod repl_meta;

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::{ParseError, Parser};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::InstructionTraceMode;
use lua_vm::runtime::Runtime;
use lua_vm::state::LuaState;

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

/// Lua diagnostics are byte strings, including at the final CLI boundary.
#[derive(Debug)]
struct CliError(Vec<u8>);

impl CliError {
    fn with_prefix(prefix: impl AsRef<[u8]>, message: &[u8]) -> Self {
        let mut bytes = prefix.as_ref().to_vec();
        bytes.extend_from_slice(message);
        Self(bytes)
    }
}

impl From<Vec<u8>> for CliError {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self(message.into_bytes())
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self(message.as_bytes().to_vec())
    }
}

impl AsRef<[u8]> for CliError {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunMode {
    Version,
    Help,
    Error,
    DefaultBehavior,
    Repl,
    Script,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupActionKind {
    ExecuteChunk,
    RequireModule,
}

#[derive(Clone, Debug)]
struct StartupAction {
    kind: StartupActionKind,
    argument: String,
}

#[derive(Clone, Debug)]
struct AppOptions {
    mode: RunMode,
    error: Option<String>,
    interactive: bool,
    script_file: Option<String>,
    script_index: Option<usize>,
    trace_file: Option<String>,
    trace_mode: InstructionTraceMode,
    startup_actions: Vec<StartupAction>,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let options = parse_args(&args);
    let program_name = program_name(args.first().map(String::as_str).unwrap_or("lua"));

    let status = match options.mode {
        RunMode::Version => {
            let _ = write_stdout_text(
                b"Lua C++ Interpreter v0.1.0\nBased on Lua 5.1.5 specification\nCopyright (C) 2025 Lua C++ Project\n",
            );
            0
        }
        RunMode::Help => {
            print_usage(args.first().map(String::as_str).unwrap_or("lua"));
            1
        }
        RunMode::Error => {
            report_error(
                program_name,
                options
                    .error
                    .as_deref()
                    .unwrap_or("unrecognized command-line option"),
            );
            1
        }
        RunMode::DefaultBehavior | RunMode::Script | RunMode::Repl => {
            run_app(&args, &options, program_name)
        }
    };

    std::process::exit(status);
}

fn parse_args(args: &[String]) -> AppOptions {
    let mut options = AppOptions {
        mode: RunMode::DefaultBehavior,
        error: None,
        interactive: false,
        script_file: None,
        script_index: None,
        trace_file: None,
        trace_mode: InstructionTraceMode::Snapshot,
        startup_actions: Vec::new(),
    };

    let mut show_version = false;
    let mut show_help = false;
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-v" {
            show_version = true;
        } else if arg == "-h" {
            show_help = true;
        } else if arg == "-i" {
            options.interactive = true;
        } else if arg == "--trace" {
            if i + 1 >= args.len() {
                options.error = Some("trace option needs file argument".to_string());
                break;
            }
            i += 1;
            options.trace_file = Some(args[i].clone());
            options.trace_mode = InstructionTraceMode::Snapshot;
        } else if arg == "--trace-diff" {
            if i + 1 >= args.len() {
                options.error = Some("trace option needs file argument".to_string());
                break;
            }
            i += 1;
            options.trace_file = Some(args[i].clone());
            options.trace_mode = InstructionTraceMode::Diff;
        } else if arg == "--" {
            if i + 1 < args.len() {
                options.script_file = Some(args[i + 1].clone());
                options.script_index = Some(i + 1);
            }
            break;
        } else if arg == "-" {
            options.script_file = Some(arg.clone());
            options.script_index = Some(i);
            break;
        } else if arg == "-e" {
            if i + 1 >= args.len() {
                options.error = Some("'-e' needs argument".to_string());
                break;
            }
            if args[i + 1] == "--" {
                options.startup_actions.push(StartupAction {
                    kind: StartupActionKind::ExecuteChunk,
                    argument: " ".to_string(),
                });
            } else {
                i += 1;
                options.startup_actions.push(StartupAction {
                    kind: StartupActionKind::ExecuteChunk,
                    argument: args[i].clone(),
                });
            }
        } else if let Some(chunk) = arg.strip_prefix("-e") {
            options.startup_actions.push(StartupAction {
                kind: StartupActionKind::ExecuteChunk,
                argument: chunk.to_string(),
            });
        } else if let Some(module) = arg.strip_prefix("-l") {
            let module = if module.is_empty() {
                if i + 1 >= args.len() {
                    options.error = Some("'-l' needs argument".to_string());
                    break;
                }
                i += 1;
                args[i].clone()
            } else {
                module.to_string()
            };
            options.startup_actions.push(StartupAction {
                kind: StartupActionKind::RequireModule,
                argument: module,
            });
        } else if !arg.starts_with('-') {
            options.script_file = Some(arg.clone());
            options.script_index = Some(i);
            break;
        } else {
            options.error = Some("unrecognized option".to_string());
            break;
        }
        i += 1;
    }

    options.mode = if show_version {
        RunMode::Version
    } else if options.error.is_some() {
        RunMode::Error
    } else if show_help {
        RunMode::Help
    } else if options.script_file.is_some() || !options.startup_actions.is_empty() {
        RunMode::Script
    } else if options.interactive {
        RunMode::Repl
    } else {
        RunMode::DefaultBehavior
    };

    options
}

fn print_usage(program: &str) {
    let usage = format!(
        concat!(
            "Usage: {} [options] [script [args]]\n",
            "Available options are:\n",
            "  -v       show version information\n",
            "  -e stat  execute string 'stat'\n",
            "  -l name  require library 'name'\n",
            "  -i       enter interactive mode\n",
            "  --trace <file>  write execution trace to JSONL file\n",
            "  --trace-diff <file>  write execution trace with changedRegisters\n",
            "  --       stop handling options\n",
            "  -        execute stdin\n",
        ),
        program
    );
    let _ = write_stdout_text(usage.as_bytes());
}

fn run_app(args: &[String], options: &AppOptions, program_name: &str) -> i32 {
    let mut runtime = match Runtime::try_new() {
        Ok(runtime) => runtime,
        Err(err) => {
            report_error(program_name, err.to_string());
            return 1;
        }
    };
    let initialized = match runtime.parts_mut() {
        Ok(mut parts) => {
            parts.with_context_gc(open_all);
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    };
    if let Err(err) = initialized {
        report_error(program_name, &err);
        return 1;
    }

    let trace_started = if let Some(path) = options.trace_file.as_deref() {
        match fs::File::create(path) {
            Ok(file) => match runtime
                .start_instruction_trace_with_mode(Box::new(file), options.trace_mode)
            {
                Ok(()) => {
                    let diagnostic = if options.trace_mode == InstructionTraceMode::Diff {
                        format!("[TRACE] Trace diff enabled → {path}\n")
                    } else {
                        format!("[TRACE] Trace enabled → {path}\n")
                    };
                    let _ = write_stdout_text(diagnostic.as_bytes());
                    true
                }
                Err(error) => {
                    report_trace_warning(&format!("cannot start trace: {error}"));
                    false
                }
            },
            Err(_) => {
                report_trace_warning(&format!("cannot open trace file: {path}"));
                false
            }
        }
    } else {
        false
    };
    let status = match run_app_with_runtime(&mut runtime, args, options) {
        Ok(()) => 0,
        Err(err) => {
            report_error(program_name, &err);
            1
        }
    };

    let trace_error = if trace_started {
        match runtime.finish_instruction_trace() {
            Ok(Some(summary)) => {
                let diagnostic = format!(
                    "[TRACE] Trace complete: {} events written.\n",
                    summary.event_count
                );
                let _ = write_stdout_text(diagnostic.as_bytes());
                None
            }
            Ok(None) => None,
            Err(error) => Some(error.to_string()),
        }
    } else {
        None
    };

    if let Err(err) = runtime.close() {
        report_error(program_name, err.to_string());
        return 1;
    }

    if let Some(error) = trace_error {
        report_error(program_name, &error);
        return 1;
    }

    status
}

fn run_app_with_runtime(
    runtime: &mut Runtime,
    args: &[String],
    options: &AppOptions,
) -> Result<(), CliError> {
    execute_startup_actions(runtime, options)?;

    if let (Some(script), Some(script_index)) = (&options.script_file, options.script_index) {
        {
            let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
            let (state, gc, string_pool) = parts.split_mut();
            install_arg_table(state, gc, string_pool, args, script_index)?;
        }
        let script_args = &args[script_index + 1..];
        let result = if script == "-" {
            let mut source = Vec::new();
            if let Err(err) = io::stdin().read_to_end(&mut source) {
                Err(CliError::from(err.to_string()))
            } else {
                execute_source(runtime, &source, "=stdin", script_args, None, false)
            }
        } else {
            execute_file(runtime, script, script_args, false)
        };
        result?;
    }

    if options.interactive || (options.mode == RunMode::Repl && options.script_file.is_none()) {
        run_interactive(runtime, InteractiveStyle::Quiet)?;
    } else if options.mode == RunMode::DefaultBehavior {
        run_interactive(runtime, InteractiveStyle::Full)?;
    }

    Ok(())
}

fn execute_startup_actions(runtime: &mut Runtime, options: &AppOptions) -> Result<(), CliError> {
    for action in &options.startup_actions {
        match action.kind {
            StartupActionKind::ExecuteChunk => {
                execute_source(
                    runtime,
                    action.argument.as_bytes(),
                    "=(command line)",
                    &[],
                    None,
                    false,
                )?;
            }
            StartupActionKind::RequireModule => {
                if Path::new(&action.argument).exists() {
                    execute_file(runtime, &action.argument, &[], false)?;
                } else {
                    let source = format!("require({})", lua_string_literal(&action.argument));
                    execute_source(
                        runtime,
                        source.as_bytes(),
                        "=(command line)",
                        &[],
                        None,
                        false,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn execute_file(
    runtime: &mut Runtime,
    filename: &str,
    args: &[String],
    display_results: bool,
) -> Result<(), CliError> {
    let bytes = fs::read(filename).map_err(|_| format!("cannot open file: {filename}"))?;
    let chunk_name = format!("@{filename}");
    execute_source(
        runtime,
        &bytes,
        &chunk_name,
        args,
        Some(filename),
        display_results,
    )
}

fn execute_source(
    runtime: &mut Runtime,
    source: &[u8],
    chunk_name: &str,
    args: &[String],
    script_path: Option<&str>,
    display_results: bool,
) -> Result<(), CliError> {
    let proto = {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        if let Some(path) = script_path {
            parts.with_context_gc(|state, gc| {
                lua_stdlib::package::add_script_directory_to_path(state, gc, &format!("@{path}"));
            });
        }
        let (_, gc, string_pool) = parts.split_mut();
        compile_or_load_proto(gc, string_pool, source, chunk_name)?
    };
    let (argument_values, argument_roots) = {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (_, gc, string_pool) = parts.split_mut();
        gc.with_publication(|transaction| {
            let mut values = Vec::with_capacity(args.len());
            let mut roots = Vec::with_capacity(args.len());
            for argument in args {
                let string = transaction
                    .intern_bytes(string_pool, argument.as_bytes())
                    .expect("Runtime StringPool belongs to the CLI collector");
                let string = transaction
                    .publish_as_explicit_root(string)
                    .expect("new CLI argument remains registered during root promotion");
                values.push(Value::String(string));
                roots.push(string);
            }
            (values, roots)
        })
    };
    let execution = runtime
        .execute_proto_with_args(proto, argument_values, |state, results| {
            if display_results {
                print_values(state, results);
            }
        })
        .map_err(|error| CliError::from(error.message_bytes().into_owned()));
    {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (_, gc, _) = parts.split_mut();
        for argument_root in argument_roots {
            gc.remove_root(argument_root);
        }
        gc.remove_root(proto);
    }
    execution
}

fn compile_or_load_proto(
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    source: &[u8],
    chunk_name: &str,
) -> Result<GcRef<Proto>, CliError> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse().map_err(|err| {
        let mut prefix = lua_vm::execute::chunk_name_for_diagnostic_bytes(chunk_name.as_bytes());
        prefix.extend_from_slice(format!(":{}: ", err.line).as_bytes());
        CliError::with_prefix(prefix, err.message_bytes())
    })?;

    gc.with_publication(|transaction| {
        let proto: Proto = CodeGenerator::new_in_publication_with_pool(transaction, string_pool)
            .generate(&chunk, chunk_name)
            .map_err(|err| {
                if err.line == 0 && err.column == 0 {
                    // The fixed C++ CLI routes location-free code-generation
                    // failures through its trusted loadstring boundary.
                    CliError::with_prefix(b"loadstring: ", err.message_bytes())
                } else {
                    let mut prefix =
                        lua_vm::execute::chunk_name_for_diagnostic_bytes(chunk_name.as_bytes());
                    prefix.extend_from_slice(format!(":{}:{}: ", err.line, err.column).as_bytes());
                    CliError::with_prefix(prefix, err.message_bytes())
                }
            })?;
        let proto = transaction.alloc(proto);
        transaction.publish_as_explicit_root(proto).map_err(|err| {
            CliError::from(format!("{chunk_name}: invalid Proto publication: {err}"))
        })
    })
}

fn install_arg_table(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    args: &[String],
    script_index: usize,
) -> Result<(), String> {
    let Some(global_table) = state.global_table else {
        return Ok(());
    };
    gc.with_publication(|transaction| {
        let global = transaction
            .protect(global_table)
            .map_err(|error| format!("invalid global-table publication owner: {error}"))?;
        let table = transaction.alloc(Table::new());
        for (idx, arg) in args.iter().enumerate() {
            let mut text = arg.clone();
            if idx == 0 {
                text = text.replace('\\', "/");
            } else if idx + 1 < script_index
                && arg == "-e"
                && args.get(idx + 1).is_some_and(|next| next == "--")
            {
                text = "-e ".to_string();
            }
            let key = Value::Number(idx as f64 - script_index as f64);
            let value = transaction
                .intern_bytes(string_pool, text.as_bytes())
                .map_err(|error| format!("invalid arg string interning: {error}"))?;
            transaction
                .set_table_entry_string(&table, &key, &value)
                .map_err(|error| format!("invalid arg-table entry publication: {error}"))?;
        }

        let name = transaction
            .intern_bytes(string_pool, b"arg")
            .map_err(|error| format!("invalid arg name interning: {error}"))?;
        transaction
            .set_table_table(&global, &name, &table)
            .map_err(|error| format!("invalid arg-table global publication: {error}"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InteractiveStyle {
    Quiet,
    Full,
}

/// Only the prepare/parse phase can request another line of source.
#[derive(Debug)]
enum ReplError {
    Syntax(ParseError),
    Terminal(CliError),
}

impl ReplError {
    fn into_diagnostic(self) -> CliError {
        match self {
            Self::Syntax(error) => {
                CliError::with_prefix(format!("stdin:{}: ", error.line), error.message_bytes())
            }
            Self::Terminal(error) => error,
        }
    }
}

impl From<CliError> for ReplError {
    fn from(error: CliError) -> Self {
        Self::Terminal(error)
    }
}

fn run_interactive(runtime: &mut Runtime, style: InteractiveStyle) -> Result<(), CliError> {
    let stdin = io::stdin();
    let mut input = String::new();
    let mut buffer = String::new();
    let mut first_line = true;
    let mut expression = false;
    let mut history = if style == InteractiveStyle::Full {
        load_repl_history()
    } else {
        Vec::new()
    };

    if style == InteractiveStyle::Full {
        write_stdout_text(b"Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio\n")
            .map_err(|err| err.to_string())?;
    }

    loop {
        let (prompt_name, default_prompt) = if first_line {
            ("_PROMPT", "> ")
        } else {
            ("_PROMPT2", ">> ")
        };
        let prompt = {
            let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
            parts.with_context(|state| global_string(state, prompt_name))
        }
        .unwrap_or_else(|| default_prompt.to_string());
        write_stdout_text(prompt.as_bytes()).map_err(|err| err.to_string())?;

        input.clear();
        let console_read = if style == InteractiveStyle::Full {
            repl_console::read_line(runtime, &prompt)?
        } else {
            None
        };
        match console_read {
            Some(repl_console::ConsoleRead::Line(line)) => input = line,
            Some(repl_console::ConsoleRead::Interrupted) => {
                buffer.clear();
                first_line = true;
                expression = false;
                continue;
            }
            Some(repl_console::ConsoleRead::Eof) => {
                write_stdout_text(b"\n").map_err(|err| err.to_string())?;
                break;
            }
            None => {
                let read = stdin.read_line(&mut input).map_err(|err| err.to_string())?;
                if read == 0 {
                    write_stdout_text(b"\n").map_err(|err| err.to_string())?;
                    break;
                }
                if input.ends_with('\n') {
                    input.pop();
                    if input.ends_with('\r') {
                        input.pop();
                    }
                }
            }
        }

        if style == InteractiveStyle::Full && input.contains('\t') {
            let completion = repl_completion::apply_submitted(runtime, &input)?;
            input = completion.line;
            if !completion.stdout.is_empty() {
                write_stdout_text(completion.stdout.as_bytes()).map_err(|err| err.to_string())?;
            }
        }

        if first_line && input.is_empty() {
            continue;
        }

        if style == InteractiveStyle::Full && !input.is_empty() {
            history.push(input.clone());
        }
        if style == InteractiveStyle::Full
            && first_line
            && matches!(input.as_str(), "exit" | "quit")
        {
            break;
        }
        if style == InteractiveStyle::Full
            && first_line
            && let Some(output) = repl_meta::run(runtime, &input)
        {
            if !output.stdout.is_empty() {
                write_stdout_text(output.stdout.as_bytes()).map_err(|err| err.to_string())?;
            }
            if !output.stderr.is_empty() {
                report_repl_error(&output.stderr);
            }
            continue;
        }

        if first_line {
            if let Some(expr) = input.strip_prefix('=') {
                buffer = format!("return {expr}");
                expression = true;
            } else {
                buffer = input.clone();
                expression = false;
            }
        } else {
            buffer.push('\n');
            buffer.push_str(&input);
        }

        match execute_repl_source(runtime, buffer.as_bytes(), expression) {
            Ok(()) => {
                buffer.clear();
                first_line = true;
                expression = false;
            }
            Err(ReplError::Syntax(ref err))
                if is_incomplete_syntax(err)
                    || (style == InteractiveStyle::Quiet
                        && first_line
                        && is_pending_assignment_name(buffer.as_bytes())) =>
            {
                first_line = false;
            }
            Err(err) => {
                report_repl_error(err.into_diagnostic());
                buffer.clear();
                first_line = true;
                expression = false;
            }
        }
    }

    if style == InteractiveStyle::Full {
        let _ = save_repl_history(&history);
        write_stdout_text(b"Goodbye!\n").map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn load_repl_history() -> Vec<String> {
    let Ok(bytes) = fs::read(".lua_history") else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn save_repl_history(history: &[String]) -> Result<(), String> {
    let mut file = fs::File::create(".lua_history").map_err(|error| error.to_string())?;
    for line in history {
        lua_stdlib::host_io::write_text(&mut file, line.as_bytes())
            .map_err(|error| error.to_string())?;
        lua_stdlib::host_io::write_text(&mut file, b"\n").map_err(|error| error.to_string())?;
    }
    file.flush().map_err(|error| error.to_string())
}

fn execute_repl_source(
    runtime: &mut Runtime,
    source: &[u8],
    display_results: bool,
) -> Result<(), ReplError> {
    let proto = {
        let mut parts = runtime
            .parts_mut()
            .map_err(|error| CliError::from(error.to_string()))?;
        let (_, gc, string_pool) = parts.split_mut();
        compile_repl_proto(gc, string_pool, source)?
    };
    let execution = runtime
        .execute_proto_with_args(proto, Vec::new(), |state, results| {
            if display_results {
                print_values(state, results);
            }
        })
        .map_err(|error| ReplError::Terminal(error.message_bytes().into_owned().into()));
    {
        let mut parts = runtime
            .parts_mut()
            .map_err(|error| CliError::from(error.to_string()))?;
        let (_, gc, _) = parts.split_mut();
        gc.remove_root(proto);
    }
    execution
}

fn compile_repl_proto(
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    source: &[u8],
) -> Result<GcRef<Proto>, ReplError> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse().map_err(ReplError::Syntax)?;

    gc.with_publication(|transaction| {
        let proto: Proto = CodeGenerator::new_in_publication_with_pool(transaction, string_pool)
            .generate(&chunk, "=(repl)")
            .map_err(|err| CliError::from(err.message_bytes().to_vec()))?;
        let proto = transaction.alloc(proto);
        transaction.publish_as_explicit_root(proto).map_err(|err| {
            ReplError::Terminal(format!("(repl): invalid Proto publication: {err}").into())
        })
    })
}

fn print_values(state: &LuaState, values: &[Value]) {
    let mut output = Vec::new();
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            output.push(b'\t');
        }
        output.extend_from_slice(value_to_string(state, value).as_bytes());
    }
    output.push(b'\n');
    let _ = write_stdout_text(&output);
}

fn value_to_string(state: &LuaState, value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(value) => value.to_string(),
        Value::Number(value) => lua_core::number_format::format_lua_number(*value),
        Value::String(value) => state
            .with_string_bytes(*value, |bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default(),
        Value::Table(value) => format!("table: {:p}", value.as_ptr()),
        Value::Function(value) => format!("function: {:p}", value.as_ptr()),
        Value::Userdata(value) => format!("userdata: {:p}", value.as_ptr()),
        Value::Thread(value) => format!("thread: {:p}", value.as_ptr()),
        Value::LightUserdata(value) => format!("userdata: {:p}", value.as_ptr()),
    }
}

fn global_string(state: &LuaState, name: &str) -> Option<String> {
    let global = state.global_table?;
    let key = state.find_interned_string(name.as_bytes()).ok().flatten()?;
    let gc = state.active_gc_ptr()?;
    // SAFETY: Runtime installs the live collector for the dynamic main-state turn.
    let Value::String(value) = unsafe { &*gc }
        .with_ref(global, |table| table.get(&Value::String(key)))
        .ok()?
    else {
        return None;
    };
    state
        .with_string_bytes(value, |bytes| String::from_utf8_lossy(bytes).into_owned())
        .ok()
}

fn is_incomplete_syntax(error: &ParseError) -> bool {
    // Fixed C++ repl_exe.cpp::isIncompleteInput checks only parse diagnostics.
    let message = error.message_bytes();
    const MARKERS: &[&[u8]] = &[
        b"<eof>",
        b"unexpected end of input",
        b"Unterminated string",
        b"Unterminated long string",
        b"Unterminated long comment",
    ];
    MARKERS.iter().any(|marker| {
        message
            .windows(marker.len())
            .any(|window| window == *marker)
    }) || [
        // Rust lexer.rs spells these same three lexer failures differently.
        // Match their diagnostic prefixes, not arbitrary quoted token content.
        b"unterminated string near '".as_slice(),
        b"unfinished long string near '",
        b"unfinished long comment near '",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

fn is_pending_assignment_name(mut source: &[u8]) -> bool {
    // Fixed C++ main.cpp trims C-locale isspace, which includes vertical tab.
    let is_space = |byte: &u8| matches!(*byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    while source.first().is_some_and(is_space) {
        source = &source[1..];
    }
    while source.last().is_some_and(is_space) {
        source = &source[..source.len() - 1];
    }
    let Some((&head, tail)) = source.split_first() else {
        return false;
    };
    (head.is_ascii_alphabetic() || head == b'_')
        && tail
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn lua_string_literal(text: &str) -> String {
    let mut result = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn program_name(argument_zero: &str) -> &str {
    argument_zero
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or("lua")
}

fn write_stdout_text(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    lua_stdlib::host_io::write_text(&mut stdout, bytes)?;
    stdout.flush()
}

fn report_error(program_name: &str, message: impl AsRef<[u8]>) {
    let mut stderr = io::stderr().lock();
    let mut diagnostic = program_name.as_bytes().to_vec();
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(message.as_ref());
    diagnostic.push(b'\n');
    let _ = lua_stdlib::host_io::write_text(&mut stderr, &diagnostic);
    let _ = stderr.flush();
}

fn report_trace_warning(message: &str) {
    let mut stderr = io::stderr().lock();
    let diagnostic = format!("[TRACE] Warning: {message}.\n");
    let _ = lua_stdlib::host_io::write_text(&mut stderr, diagnostic.as_bytes());
    let _ = stderr.flush();
}

fn report_repl_error(message: impl AsRef<[u8]>) {
    let mut stderr = io::stderr().lock();
    let _ = lua_stdlib::host_io::write_text(&mut stderr, message.as_ref());
    let _ = lua_stdlib::host_io::write_text(&mut stderr, b"\n");
    let _ = stderr.flush();
}

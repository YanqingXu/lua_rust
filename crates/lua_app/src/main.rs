//! lua_app: Lua 5.1 command-line runner.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::runtime::Runtime;
use lua_vm::state::LuaState;

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunMode {
    Version,
    Help,
    Error,
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
    startup_actions: Vec<StartupAction>,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let options = parse_args(&args);

    let status = match options.mode {
        RunMode::Version => {
            println!("Lua 5.1 Rust Interpreter");
            0
        }
        RunMode::Help => {
            print_usage(args.first().map(String::as_str).unwrap_or("lua"));
            1
        }
        RunMode::Error => {
            eprintln!(
                "{}",
                options
                    .error
                    .as_deref()
                    .unwrap_or("unrecognized command-line option")
            );
            1
        }
        RunMode::Script | RunMode::Repl => run_app(&args, &options),
    };

    std::process::exit(status);
}

fn parse_args(args: &[String]) -> AppOptions {
    let mut options = AppOptions {
        mode: RunMode::Repl,
        error: None,
        interactive: false,
        script_file: None,
        script_index: None,
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
    } else {
        RunMode::Repl
    };

    options
}

fn print_usage(program: &str) {
    println!("Usage: {program} [options] [script [args]]");
    println!("Available options are:");
    println!("  -v       show version information");
    println!("  -e stat  execute string 'stat'");
    println!("  -l name  require library 'name'");
    println!("  -i       enter interactive mode");
    println!("  --       stop handling options");
    println!("  -        execute stdin");
}

fn run_app(args: &[String], options: &AppOptions) -> i32 {
    let mut runtime = match Runtime::try_new() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let initialized = match runtime.parts_mut() {
        Ok(mut parts) => {
            let (state, gc, string_pool) = parts.split_mut();
            let _ = string_pool;
            open_all(state, gc);
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    };
    if let Err(err) = initialized {
        eprintln!("{err}");
        return 1;
    }
    let status = match run_app_with_runtime(&mut runtime, args, options) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    };

    if let Err(err) = runtime.close() {
        eprintln!("{err}");
        return 1;
    }

    status
}

fn run_app_with_runtime(
    runtime: &mut Runtime,
    args: &[String],
    options: &AppOptions,
) -> Result<(), String> {
    execute_startup_actions(runtime, options)?;

    if let (Some(script), Some(script_index)) = (&options.script_file, options.script_index) {
        {
            let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
            let (state, gc, _) = parts.split_mut();
            install_arg_table(state, gc, args, script_index)?;
        }
        let script_args = &args[script_index + 1..];
        let result = if script == "-" {
            let mut source = Vec::new();
            if let Err(err) = io::stdin().read_to_end(&mut source) {
                Err(err.to_string())
            } else {
                execute_source(runtime, &source, "=stdin", script_args, None, false)
            }
        } else {
            execute_file(runtime, script, script_args, false)
        };
        result?;
    }

    if (options.interactive || (options.mode == RunMode::Repl && options.script_file.is_none()))
        && let Err(err) = run_quiet_interactive(runtime)
    {
        return Err(err);
    }

    Ok(())
}

fn execute_startup_actions(runtime: &mut Runtime, options: &AppOptions) -> Result<(), String> {
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
) -> Result<(), String> {
    let bytes = fs::read(filename).map_err(|err| format!("cannot open {filename}: {err}"))?;
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
) -> Result<(), String> {
    let proto = {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (state, gc, string_pool) = parts.split_mut();
        if let Some(path) = script_path {
            lua_stdlib::package::add_script_directory_to_path(state, gc, &format!("@{path}"));
        }
        compile_or_load_proto(gc, string_pool, source, chunk_name)?
    };
    let (argument_values, argument_roots) = {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (_, gc, _) = parts.split_mut();
        gc.with_publication(|transaction| {
            let mut values = Vec::with_capacity(args.len());
            let mut roots = Vec::with_capacity(args.len());
            for argument in args {
                let string = transaction.alloc(GcString::from_utf8_text(argument));
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
        .execute_proto_with_args(proto, argument_values, |results| {
            if display_results {
                print_values(results);
            }
        })
        .map_err(|error| error.to_string());
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
) -> Result<GcRef<Proto>, String> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("{chunk_name}:{}: {}", err.line, err.message))?;

    gc.with_publication(|transaction| {
        let proto: Proto = CodeGenerator::new_in_publication_with_pool(transaction, string_pool)
            .generate(&chunk, chunk_name)
            .map_err(|err| format!("{chunk_name}:{err}"))?;
        let proto = transaction.alloc(proto);
        transaction
            .publish_as_explicit_root(proto)
            .map_err(|err| format!("{chunk_name}: invalid Proto publication: {err}"))
    })
}

fn install_arg_table(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
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
            let value = transaction.alloc(GcString::from_utf8_text(&text));
            transaction
                .set_table_entry_string(&table, &key, &value)
                .map_err(|error| format!("invalid arg-table entry publication: {error}"))?;
        }

        let name = transaction.alloc(GcString::from_bytes(b"arg"));
        transaction
            .set_table_table(&global, &name, &table)
            .map_err(|error| format!("invalid arg-table global publication: {error}"))
    })
}

fn run_quiet_interactive(runtime: &mut Runtime) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = String::new();
    let mut buffer = String::new();
    let mut first_line = true;
    let mut expression = false;

    loop {
        let prompt = if first_line {
            runtime
                .main_state()
                .and_then(|state| global_string(state, "_PROMPT"))
                .unwrap_or_else(|| "> ".to_string())
        } else {
            runtime
                .main_state()
                .and_then(|state| global_string(state, "_PROMPT2"))
                .unwrap_or_else(|| ">> ".to_string())
        };
        print!("{prompt}");
        io::stdout().flush().map_err(|err| err.to_string())?;

        input.clear();
        let read = stdin.read_line(&mut input).map_err(|err| err.to_string())?;
        if read == 0 {
            println!();
            return Ok(());
        }
        if input.ends_with('\n') {
            input.pop();
            if input.ends_with('\r') {
                input.pop();
            }
        }

        if first_line && input.is_empty() {
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

        match execute_source(runtime, buffer.as_bytes(), "=stdin", &[], None, expression) {
            Ok(()) => {
                buffer.clear();
                first_line = true;
                expression = false;
            }
            Err(err)
                if is_incomplete_error(&err) || (first_line && is_pending_assignment(&buffer)) =>
            {
                first_line = false;
            }
            Err(err) => {
                return Err(err);
            }
        }
    }
}

fn print_values(values: &[Value]) {
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            print!("\t");
        }
        print!("{}", value_to_string(value));
    }
    println!();
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(value) => value.to_string(),
        Value::Number(value) => {
            if value.fract() == 0.0 && value.is_finite() {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        Value::String(value) => {
            // SAFETY: values being printed are returned on the live Lua stack.
            unsafe { value.as_ref() }
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
        Value::Table(value) => format!("table: {:p}", value.as_ptr()),
        Value::Function(value) => format!("function: {:p}", value.as_ptr()),
        Value::Userdata(value) => format!("userdata: {:p}", value.as_ptr()),
        Value::Thread(value) => format!("thread: {:p}", value.as_ptr()),
        Value::LightUserdata(value) => format!("userdata: {:p}", value.as_ptr()),
    }
}

fn global_string(state: &LuaState, name: &str) -> Option<String> {
    let global = state.global_table?;
    // SAFETY: the global table is rooted by the LuaState.
    let table = unsafe { global.as_ref() }?;
    for (key, value) in table.hash_entries() {
        if let Value::String(key_ref) = key
            // SAFETY: the key is held by the global table.
            && let Some(key_string) = unsafe { key_ref.as_ref() }
            && key_string.as_bytes() == name.as_bytes()
            && let Value::String(value_ref) = value
        {
            // SAFETY: the value is held by the global table.
            return unsafe { value_ref.as_ref() }.map(|value| value.to_string_lossy().into_owned());
        }
    }
    None
}

fn is_incomplete_error(message: &str) -> bool {
    message.contains("<eof>")
        || message.contains("to close")
        || message.contains("unterminated string")
        || message.contains("unfinished string")
        || message.contains("unfinished long string")
        || message.contains("unfinished long comment")
}

fn is_pending_assignment(source: &str) -> bool {
    let trimmed = source.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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

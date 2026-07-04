//! lua_app: Lua 5.1 command-line runner.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::execute::call_value;
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
    let mut gc = GarbageCollector::new();
    let mut string_pool = StringPool::new();
    let global_table = gc.create_root(Table::new());
    let mut state = LuaState::with_global_table(global_table);
    state.string_pool = Some(&mut string_pool as *mut StringPool);
    open_all(&mut state, &mut gc);

    if let Err(err) = execute_startup_actions(&mut state, &mut gc, &mut string_pool, options) {
        eprintln!("{err}");
        return 1;
    }

    if let (Some(script), Some(script_index)) = (&options.script_file, options.script_index) {
        install_arg_table(&mut state, &mut gc, args, script_index);
        let script_args = lua_script_args(&mut gc, args, script_index);
        let result = if script == "-" {
            let mut source = String::new();
            if let Err(err) = io::stdin().read_to_string(&mut source) {
                Err(err.to_string())
            } else {
                execute_source(
                    &mut state,
                    &mut gc,
                    &mut string_pool,
                    &source,
                    "=stdin",
                    &script_args,
                    None,
                )
                .map(|_| ())
            }
        } else {
            execute_file(&mut state, &mut gc, &mut string_pool, script, &script_args).map(|_| ())
        };
        if let Err(err) = result {
            eprintln!("{err}");
            return 1;
        }
    }

    if options.interactive || (options.mode == RunMode::Repl && options.script_file.is_none()) {
        if let Err(err) = run_quiet_interactive(&mut state, &mut gc, &mut string_pool) {
            eprintln!("{err}");
            return 1;
        }
    }

    0
}

fn execute_startup_actions(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    options: &AppOptions,
) -> Result<(), String> {
    for action in &options.startup_actions {
        match action.kind {
            StartupActionKind::ExecuteChunk => {
                execute_source(
                    state,
                    gc,
                    string_pool,
                    &action.argument,
                    "=(command line)",
                    &[],
                    None,
                )?;
            }
            StartupActionKind::RequireModule => {
                if Path::new(&action.argument).exists() {
                    execute_file(state, gc, string_pool, &action.argument, &[])?;
                } else {
                    let source = format!("require({})", lua_string_literal(&action.argument));
                    execute_source(
                        state,
                        gc,
                        string_pool,
                        &source,
                        "=(command line)",
                        &[],
                        None,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn execute_file(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    filename: &str,
    args: &[Value],
) -> Result<Vec<Value>, String> {
    let bytes = fs::read(filename).map_err(|err| format!("cannot open {filename}: {err}"))?;
    let source = lua_source_from_bytes(&bytes);
    let chunk_name = format!("@{filename}");
    execute_source(
        state,
        gc,
        string_pool,
        &source,
        &chunk_name,
        args,
        Some(filename),
    )
}

fn execute_source(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    source: &str,
    chunk_name: &str,
    args: &[Value],
    script_path: Option<&str>,
) -> Result<Vec<Value>, String> {
    if let Some(path) = script_path {
        lua_stdlib::package::add_script_directory_to_path(state, gc, &format!("@{path}"));
    }

    let function_ref = compile_or_load_function(state, gc, string_pool, source, chunk_name)?;
    call_value(state, gc, Value::Function(function_ref), args, None).map_err(|err| err.to_string())
}

fn compile_or_load_function(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    source: &str,
    chunk_name: &str,
) -> Result<GcRef<Function>, String> {
    let load_source = skip_initial_hash_comment_line(source);
    if let Some(result) = lua_stdlib::dump::undump_function(state, gc, load_source) {
        return result;
    }

    let mut parser = Parser::new(source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("{chunk_name}:{}: {}", err.line, err.message))?;

    let mut cg = CodeGenerator::new(gc);
    cg.builder.bind_pool(string_pool);
    let proto: Proto = cg
        .generate(&chunk, chunk_name)
        .map_err(|err| format!("{chunk_name}:{err}"))?;
    let proto_ref = gc.create(proto);
    let mut function = Function::new_lua(proto_ref);
    function.set_env(state.thread_env.or(state.global_table));
    Ok(gc.create(function))
}

fn install_arg_table(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    args: &[String],
    script_index: usize,
) {
    let mut table = Table::new();
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
        let value = Value::String(gc.create(GcString::new(&text)));
        table.set(&key, &value);
    }

    let arg_ref = gc.create(table);
    if let Some(global_table) = state.global_table {
        let name = gc.create(GcString::new("arg"));
        let global_ptr = global_table.as_ptr() as *mut Table;
        // SAFETY: the global table is a GC root owned by this LuaState.
        unsafe {
            (*global_ptr).set(&Value::String(name), &Value::Table(arg_ref));
        }
    }
}

fn lua_script_args(gc: &mut GarbageCollector, args: &[String], script_index: usize) -> Vec<Value> {
    args.iter()
        .skip(script_index + 1)
        .map(|arg| Value::String(gc.create(GcString::new(arg))))
        .collect()
}

fn run_quiet_interactive(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = String::new();
    let mut buffer = String::new();
    let mut first_line = true;
    let mut expression = false;

    loop {
        let prompt = if first_line {
            global_string(state, "_PROMPT").unwrap_or_else(|| "> ".to_string())
        } else {
            global_string(state, "_PROMPT2").unwrap_or_else(|| ">> ".to_string())
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

        match execute_source(state, gc, string_pool, &buffer, "=stdin", &[], None) {
            Ok(results) => {
                if expression {
                    print_values(&results);
                }
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
                .map(|value| value.data().to_string())
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
            && key_string.data() == name
            && let Value::String(value_ref) = value
        {
            // SAFETY: the value is held by the global table.
            return unsafe { value_ref.as_ref() }.map(|value| value.data().to_string());
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

fn lua_source_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn skip_initial_hash_comment_line(source: &str) -> &str {
    if !source.starts_with('#') {
        return source;
    }
    let Some(newline) = source.find(['\r', '\n']) else {
        return source;
    };
    source[newline..].trim_start_matches(['\r', '\n'])
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

use lua_app::{compile_and_disassemble, AppError, Session};
use lua_core::Value;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::process::ExitCode;

enum Action {
    Execute(Vec<u8>),
    Load(Vec<u8>),
    Version,
}

struct Options {
    actions: Vec<Action>,
    interactive: bool,
    dump_bytecode: bool,
    script: Option<OsString>,
    script_position: Option<usize>,
    raw_arguments: Vec<OsString>,
}

enum CliError {
    Usage(String),
    Runtime(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprintln!("lua: {message}");
            eprintln!("usage: lua_app [-i] [-v] [-e stat] [-l name] [--] [script [args]]");
            ExitCode::from(2)
        }
        Err(CliError::Runtime(message)) => {
            eprintln!("lua: {message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), CliError> {
    let options = parse_options(env::args_os().collect())?;
    let mut session = Session::new();
    session
        .set_arg_table(&arg_entries(&options))
        .map_err(runtime_error)?;

    for action in &options.actions {
        match action {
            Action::Execute(source) => {
                session.execute_multi(source).map_err(runtime_error)?;
            }
            Action::Load(module) => session.require(module).map_err(runtime_error)?,
            Action::Version => println!("Lua 5.1 (lua-rust teaching v0.18)"),
        }
    }

    if let Some(path) = &options.script {
        let source = if path == "-" {
            read_stdin().map_err(CliError::Runtime)?
        } else {
            fs::read(path)
                .map_err(|error| CliError::Runtime(format!("cannot read {path:?}: {error}")))?
        };
        if options.dump_bytecode {
            print!(
                "{}",
                compile_and_disassemble(&source).map_err(runtime_error)?
            );
        }
        session.execute_multi(&source).map_err(runtime_error)?;
    } else if options.actions.is_empty() && !options.interactive {
        if io::stdin().is_terminal() {
            return repl(&mut session, true);
        }
        let source = read_stdin().map_err(CliError::Runtime)?;
        session.execute_multi(&source).map_err(runtime_error)?;
    }

    if options.interactive {
        repl(&mut session, true)?;
    }
    Ok(())
}

fn parse_options(raw_arguments: Vec<OsString>) -> Result<Options, CliError> {
    let mut actions = Vec::new();
    let mut interactive = false;
    let mut dump_bytecode = false;
    let mut script = None;
    let mut script_position = None;
    let mut index = 1;
    let mut stop_options = false;
    while index < raw_arguments.len() {
        let argument = &raw_arguments[index];
        let text = argument.to_string_lossy();
        if !stop_options && text == "--" {
            stop_options = true;
            index += 1;
            continue;
        }
        if !stop_options && (text == "--help" || text == "-h") {
            println!("usage: lua_app [-i] [-v] [-e stat] [-l name] [--] [script [args]]");
            std::process::exit(0);
        }
        if !stop_options && text == "--dump-bytecode" {
            dump_bytecode = true;
            index += 1;
            continue;
        }
        if !stop_options && text == "-i" {
            interactive = true;
            index += 1;
            continue;
        }
        if !stop_options && (text == "-v" || text == "--version") {
            actions.push(Action::Version);
            index += 1;
            continue;
        }
        if !stop_options && (text == "-e" || text.starts_with("-e")) {
            let source = option_value(&raw_arguments, &mut index, "-e", &text)?;
            actions.push(Action::Execute(os_bytes(&source)));
            continue;
        }
        if !stop_options && (text == "-l" || text.starts_with("-l")) {
            let module = option_value(&raw_arguments, &mut index, "-l", &text)?;
            actions.push(Action::Load(os_bytes(&module)));
            continue;
        }
        if !stop_options && text.starts_with('-') && text != "-" {
            return Err(CliError::Usage(format!("unrecognized option '{text}'")));
        }
        script_position = Some(index);
        script = Some(argument.clone());
        break;
    }
    Ok(Options {
        actions,
        interactive,
        dump_bytecode,
        script,
        script_position,
        raw_arguments,
    })
}

fn option_value(
    arguments: &[OsString],
    index: &mut usize,
    option: &str,
    text: &str,
) -> Result<OsString, CliError> {
    if text.len() > option.len() {
        let value = &text[option.len()..];
        *index += 1;
        return Ok(OsString::from(value));
    }
    *index += 1;
    let value = arguments
        .get(*index)
        .ok_or_else(|| CliError::Usage(format!("'{option}' needs an argument")))?;
    *index += 1;
    Ok(value.clone())
}

fn arg_entries(options: &Options) -> Vec<(i64, Vec<u8>)> {
    let origin = options.script_position.unwrap_or(0) as i64;
    options
        .raw_arguments
        .iter()
        .enumerate()
        .map(|(index, value)| (index as i64 - origin, os_bytes(value)))
        .collect()
}

fn repl(session: &mut Session, prompts: bool) -> Result<(), CliError> {
    if prompts {
        println!("Lua 5.1 (lua-rust teaching v0.18)");
    }
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut source = String::new();
    loop {
        if prompts {
            print!("{}", if source.is_empty() { "> " } else { ">> " });
            io::stdout()
                .flush()
                .map_err(|error| CliError::Runtime(error.to_string()))?;
        }
        let Some(line) = lines.next() else {
            if prompts {
                println!();
            }
            break;
        };
        let line = line.map_err(|error| CliError::Runtime(error.to_string()))?;
        if source.is_empty() && line.trim().is_empty() {
            continue;
        }
        if source.is_empty() && line.trim_start().starts_with('=') {
            source.push_str("return ");
            source.push_str(line.trim_start().trim_start_matches('='));
        } else {
            source.push_str(&line);
        }
        source.push('\n');
        match session.execute_multi(source.as_bytes()) {
            Ok(values) => {
                print_values(&values).map_err(CliError::Runtime)?;
                source.clear();
            }
            Err(AppError::Compile(error)) if is_incomplete(&error) => continue,
            Err(error) => {
                eprintln!("lua: {error}");
                source.clear();
            }
        }
    }
    Ok(())
}

fn is_incomplete(error: &lua_compiler::CompileError) -> bool {
    error.message.contains("expected 'end'")
        || error.message.contains("unexpected end")
        || error.message.contains("unterminated")
}

fn print_values(values: &[Value]) -> Result<(), String> {
    if values.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout().lock();
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            stdout.write_all(b"\t").map_err(|error| error.to_string())?;
        }
        match value {
            Value::String(bytes) => stdout
                .write_all(bytes.as_bytes())
                .map_err(|error| error.to_string())?,
            other => stdout
                .write_all(other.to_string().as_bytes())
                .map_err(|error| error.to_string())?,
        }
    }
    stdout.write_all(b"\n").map_err(|error| error.to_string())
}

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut source = Vec::new();
    io::stdin()
        .read_to_end(&mut source)
        .map_err(|error| format!("cannot read stdin: {error}"))?;
    Ok(source)
}

fn os_bytes(value: impl AsRef<OsStr>) -> Vec<u8> {
    value.as_ref().to_string_lossy().as_bytes().to_vec()
}

fn runtime_error(error: AppError) -> CliError {
    CliError::Runtime(error.to_string())
}

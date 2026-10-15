use lua_app::{compile_and_disassemble, run_repl, AppError, Session};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::process::ExitCode;

enum Action {
    Execute(Vec<u8>),
    Load(Vec<u8>),
    Version,
    Help,
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
            Action::Version => println!("Lua 5.1 (lua-rust teaching v1.0)"),
            Action::Help => {
                println!("usage: lua_app [-i] [-v] [-e stat] [-l name] [--] [script [args]]");
            }
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
            actions.push(Action::Help);
            index += 1;
            continue;
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
    let stdin = io::stdin();
    run_repl(
        session,
        stdin.lock(),
        io::stdout().lock(),
        io::stderr().lock(),
        prompts,
    )
    .map_err(runtime_error)
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

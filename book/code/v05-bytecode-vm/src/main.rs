use lua_app::{compile_and_disassemble, execute};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("lua-step: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut dump = false;
    let mut path = None;
    for argument in env::args_os().skip(1) {
        if argument == "--dump-bytecode" {
            dump = true;
        } else if path.replace(argument).is_some() {
            return Err("usage: lua_app [--dump-bytecode] [expression.lua]".to_owned());
        }
    }
    let source = if let Some(path) = path {
        fs::read(&path).map_err(|error| format!("cannot read {path:?}: {error}"))?
    } else {
        let mut source = Vec::new();
        io::stdin()
            .read_to_end(&mut source)
            .map_err(|error| format!("cannot read stdin: {error}"))?;
        source
    };
    if dump {
        print!(
            "{}",
            compile_and_disassemble(&source).map_err(|error| error.to_string())?
        );
    }
    let value = execute(&source).map_err(|error| error.to_string())?;
    println!("type={} value={value}", value.type_name());
    Ok(())
}

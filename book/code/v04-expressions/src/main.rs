use lua_step_v04::execute;
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
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let source = match arguments.as_slice() {
        [] => {
            let mut source = Vec::new();
            io::stdin()
                .read_to_end(&mut source)
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            source
        }
        [path] => fs::read(path).map_err(|error| format!("cannot read {:?}: {error}", path))?,
        _ => return Err("usage: lua-step [expression.lua]".to_owned()),
    };
    let value = execute(&source).map_err(|error| error.to_string())?;
    println!("type={} value={value}", value.type_name());
    Ok(())
}

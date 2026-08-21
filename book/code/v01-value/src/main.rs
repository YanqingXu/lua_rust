use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

use lua_step_v01::evaluate_literal;

fn main() -> ExitCode {
    match read_source()
        .and_then(|source| evaluate_literal(&source).map_err(|error| error.to_string()))
    {
        Ok(value) => {
            println!("type={} value={}", value.type_name(), value.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("lua-step: {error}");
            ExitCode::from(2)
        }
    }
}

fn read_source() -> Result<Vec<u8>, String> {
    let mut args = env::args_os().skip(1);
    let path = args.next();
    if args.next().is_some() {
        return Err("expected at most one script path".to_owned());
    }
    match path {
        Some(path) => fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.to_string_lossy())),
        None => {
            let mut bytes = Vec::new();
            io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            Ok(bytes)
        }
    }
}

use lua_bytecode::{compatibility::opcode_audit_json, render, trace_json, ViewFormat};
use lua_compiler::compile;
use lua_vm::Vm;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!("lua-bytecode: {message}");
            ExitCode::from(code)
        }
    }
}

fn run() -> Result<(), (u8, String)> {
    let mut format = "text".to_owned();
    let mut path = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let argument_text = argument.to_string_lossy();
        if argument_text == "--help" || argument_text == "-h" {
            println!("usage: lua_bytecode [--format text|json|cfg|trace|opcode-audit] [file|-]");
            return Ok(());
        }
        if argument_text == "--format" {
            format = arguments
                .next()
                .ok_or_else(|| (2, "--format needs a value".to_owned()))?
                .to_string_lossy()
                .into_owned();
        } else if let Some(value) = argument_text.strip_prefix("--format=") {
            format = value.to_owned();
        } else if path.replace(argument).is_some() {
            return Err((2, "only one input file is allowed".to_owned()));
        }
    }
    if format == "opcode-audit" {
        if path.is_some() {
            return Err((2, "opcode-audit does not accept an input file".to_owned()));
        }
        print!("{}", opcode_audit_json());
        return Ok(());
    }
    let source = match path {
        Some(path) if path != "-" => {
            fs::read(&path).map_err(|error| (1, format!("cannot read {path:?}: {error}")))?
        }
        _ => {
            let mut source = Vec::new();
            io::stdin()
                .read_to_end(&mut source)
                .map_err(|error| (1, format!("cannot read stdin: {error}")))?;
            source
        }
    };
    let proto = compile(&source).map_err(|error| (1, error.to_string()))?;
    match format.as_str() {
        "text" => print!(
            "{}",
            render(&proto, ViewFormat::Text).map_err(|error| (1, error))?
        ),
        "json" => print!(
            "{}",
            render(&proto, ViewFormat::Json).map_err(|error| (1, error))?
        ),
        "cfg" => print!(
            "{}",
            render(&proto, ViewFormat::Cfg).map_err(|error| (1, error))?
        ),
        "trace" => {
            let mut vm = Vm::new();
            vm.enable_trace();
            vm.execute_multi(&proto)
                .map_err(|error| (1, error.to_string()))?;
            print!("{}", trace_json(&vm.take_trace()));
        }
        _ => return Err((2, format!("unknown format '{format}'"))),
    }
    Ok(())
}

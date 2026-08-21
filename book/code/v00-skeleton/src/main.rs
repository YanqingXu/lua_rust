use std::env;
use std::io;
use std::process::ExitCode;

use lua_step_v00::{load_source, run};

fn main() -> ExitCode {
    match load_source(env::args_os().skip(1), io::stdin().lock()) {
        Ok(chunk) => {
            let summary = run(&chunk);
            println!("chunk={} bytes={}", summary.chunk_name, summary.byte_count);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("lua-step: {error}");
            ExitCode::from(2)
        }
    }
}

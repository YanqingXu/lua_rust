//! Bounded protocol fixture. It starts no descendant processes and never spams indefinitely.
mod protocol;

use std::io::{Read, Write};
use std::process::ExitCode;

fn write_flush(writer: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    writer
        .write_all(bytes)
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

// ANCHOR: runner_fixture_stdin
fn stdin_mode() -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut stdin = std::io::stdin().lock();
    write_flush(&mut stdout, protocol::STDIN_READY)?;
    let mut first = [0];
    let size = stdin.read(&mut first).map_err(|error| error.to_string())?;
    write_flush(&mut stdout, protocol::STDIN_ACK)?;
    let mut input = first[..size].to_vec();
    stdin
        .read_to_end(&mut input)
        .map_err(|error| error.to_string())?;
    write_flush(&mut stdout, b"INPUT:")?;
    write_flush(&mut stdout, &input)?;
    write_flush(&mut stderr, protocol::STDIN_EOF)
}
// ANCHOR_END: runner_fixture_stdin

// ANCHOR: runner_fixture_dual
fn dual_mode() -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    write_flush(&mut stdout, protocol::DUAL_READY)?;
    let mut token = [0];
    std::io::stdin()
        .read_exact(&mut token)
        .map_err(|error| error.to_string())?;
    write_flush(&mut stdout, protocol::DUAL_ACK)?;
    // Sequential stdout-to-EOF readers now wait while this stderr write can fill.
    write_flush(&mut stderr, &protocol::stderr_payload())?;
    write_flush(&mut stdout, &protocol::stdout_payload())
}
// ANCHOR_END: runner_fixture_dual

fn run() -> Result<u8, String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.first().map(String::as_str) {
        Some("version") if arguments.len() == 1 => {
            write_flush(&mut std::io::stdout().lock(), protocol::VERSION)?;
            Ok(0)
        }
        Some("stdin") if arguments.len() == 1 => {
            stdin_mode()?;
            Ok(0)
        }
        Some("dual") if arguments.len() == 1 => {
            dual_mode()?;
            Ok(0)
        }
        Some("stall") if arguments.len() == 1 => {
            write_flush(&mut std::io::stdout().lock(), protocol::STALL_STDOUT)?;
            write_flush(&mut std::io::stderr().lock(), protocol::STALL_STDERR)?;
            std::io::stdin()
                .read_to_end(&mut Vec::new())
                .map_err(|error| error.to_string())?;
            Ok(0)
        }
        Some("exit") if arguments.len() == 2 => {
            let code = arguments[1]
                .parse::<u8>()
                .map_err(|error| error.to_string())?;
            write_flush(&mut std::io::stdout().lock(), protocol::EXIT_STDOUT)?;
            write_flush(&mut std::io::stderr().lock(), protocol::EXIT_STDERR)?;
            Ok(code)
        }
        _ => Err("usage: runner_fixture stdin|dual|stall|exit <0..255>|version".to_owned()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("runner-fixture: {error}");
            ExitCode::from(2)
        }
    }
}

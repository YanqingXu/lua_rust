//! Fixed faults/reference plus real sibling process; no nested cargo or starter.
#[path = "../exercises/cli_exit/faults.rs"]
mod faults;
#[path = "../exercises/cli_exit/model.rs"]
mod model;
#[path = "../exercises/support/process.rs"]
mod process;
#[path = "../exercises/cli_exit/real.rs"]
mod real;
#[path = "../exercises/cli_exit/solution.rs"]
mod solution;
#[path = "../exercises/support/temp.rs"]
mod temp;

use lua_core::Value;
use model::{CliError, Finish, Outcome, Transcript};

fn display(label: &str, result: Transcript) {
    println!(
        "{label}: exit={} stdout={:?} stderr={:?}",
        result.code,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn error() {
    for (name, failure) in [
        ("usage", CliError::Usage("bad option".to_owned())),
        ("runtime", CliError::Runtime("broken".to_owned())),
    ] {
        for (label, hook) in [
            (
                "before print-only",
                faults::print_error_but_succeed as Finish,
            ),
            ("before all-one", faults::all_errors_one as Finish),
            ("after classified", solution::finish as Finish),
        ] {
            display(
                &format!("{label} {name}"),
                hook(Outcome {
                    stdout: b"kept\n".to_vec(),
                    result: Err(failure.clone()),
                }),
            );
        }
    }
}

fn streams() {
    for (label, hook) in [
        (
            "before print-return",
            faults::print_script_results as Finish,
        ),
        ("after silent-return", solution::finish as Finish),
    ] {
        display(
            label,
            hook(Outcome {
                stdout: b"kept\n".to_vec(),
                result: Ok(vec![Value::Number(42.0)]),
            }),
        );
    }
}

fn real() -> Result<(), String> {
    let binary = process::example_binary()?;
    for probes in [
        real::help_version_and_usage(&binary)?,
        real::compile_runtime_file_and_silent_returns(&binary)?,
        real::failed_action_skips_later_execute_script_and_repl(&binary)?,
        real::failed_script_or_file_skips_repl(&binary)?,
    ] {
        for probe in probes {
            println!(
                "real {}: exit={} stdout={:?} stderr={}",
                probe.name,
                probe.code,
                String::from_utf8_lossy(&probe.stdout),
                probe.stderr_class
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "error" => error(),
        "streams" => streams(),
        "real" => real()?,
        "all" => {
            error();
            streams();
            real()?;
        }
        _ => return Err("usage: cli_exit_problem_chain [error|streams|real|all]".to_owned()),
    }
    Ok(())
}

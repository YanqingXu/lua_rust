#[path = "../exercises/repl_recovery/faults.rs"]
mod faults;
#[path = "../exercises/repl_recovery/model.rs"]
pub mod model;
#[path = "../exercises/support/process.rs"]
pub mod process;
#[path = "../exercises/repl_recovery/real.rs"]
pub mod real;
#[path = "../exercises/repl_recovery/solution.rs"]
mod solution;

use lua_app::AppError;

fn trace(label: &str, submit: model::Submit, lines: &[&str]) -> model::Repl {
    let mut repl = model::Repl::new(submit);
    println!("{label}");
    for line in lines {
        let event = repl.push(line);
        println!(
            "  line={line:?}; {}; buffer={:?}",
            event.describe(),
            repl.source
        );
    }
    repl
}

fn continuation() {
    let lines = ["saved=40", "if false then", "saved=99", "end", "=saved"];
    trace("before: clear every line", faults::clear_every_line, &lines);
    trace(
        "after: preserve incomplete source",
        solution::submit,
        &lines,
    );
}

fn errors() -> Result<(), String> {
    let lines = [
        "saved=40",
        "local =",
        "=saved+2",
        "error('recover')",
        "=saved",
    ];
    trace(
        "before: retain erroneous source",
        faults::keep_after_error,
        &lines,
    );
    trace("after: discard erroneous source", solution::submit, &lines);
    let runtime = [
        "effects=0",
        "effects=effects+1;error('recover')",
        "return effects",
    ];
    let mut repeated = trace(
        "before: retained runtime source repeats effects",
        faults::keep_after_error,
        &runtime,
    );
    let values = repeated
        .inspect(b"return effects")
        .map_err(|error| error.to_string())?;
    println!("  direct Session observation outside polluted buffer: effects={values:?}");
    trace(
        "after: runtime error clears source but preserves effects",
        solution::submit,
        &runtime,
    );
    Ok(())
}

fn early_return() -> Result<(), String> {
    let lines = ["saved=40", "error('recover')", "saved=42", "=saved"];
    let mut repl = model::Repl::new(solution::submit);
    let stopped = faults::stop_at_first_error(&mut repl, &lines).is_err();
    let values = repl
        .inspect(b"return saved")
        .map_err(|error| error.to_string())?;
    println!(
        "before: early return stopped={stopped} submitted={} saved={values:?}",
        repl.submitted
    );
    trace(
        "after: error is a displayed event, loop continues",
        solution::submit,
        &lines,
    );
    Ok(())
}

fn heuristic() -> Result<(), String> {
    let error = lua_compiler::compile(b"return 40 +\n").unwrap_err();
    let mut repl = model::Repl::new(solution::submit);
    let ordinary_error = matches!(
        repl.push("return 40 +"),
        model::Event::Error(AppError::Compile(_))
    );
    println!(
        "heuristic: message={:?} incomplete={} appended_chunk_compiles={} ordinary_error={} buffer_empty={}",
        error.message, model::is_incomplete(&error),
        lua_compiler::compile(b"return 40 +\n2\n").is_ok(), ordinary_error, repl.source.is_empty()
    );
    Ok(())
}

fn real_cases() -> Result<(), String> {
    let binary = process::example_binary()?;
    for case in real::interactive_recovery(&binary)?
        .into_iter()
        .chain(real::mode_boundaries(&binary)?)
    {
        println!(
            "real {}: exit={} stdout={:?} errors={}",
            case.name,
            case.exit,
            String::from_utf8_lossy(&case.stdout),
            String::from_utf8_lossy(&case.stderr)
                .matches("lua: ")
                .count()
        );
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "continuation" => { continuation(); Ok(()) }
        "errors" => errors(),
        "early-return" => early_return(),
        "heuristic" => heuristic(),
        "real" => real_cases(),
        "all" => { continuation(); errors()?; early_return()?; heuristic()?; real_cases() }
        _ => Err("usage: repl_recovery_problem_chain [continuation|errors|early-return|heuristic|real|all]".into()),
    }
}

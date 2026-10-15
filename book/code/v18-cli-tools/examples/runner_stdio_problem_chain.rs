//! Fixed pipe/EOF experiments. Every path owns, reaps and joins its exact child.
#[path = "../exercises/runner_stdio/faults.rs"]
mod faults;
#[path = "../exercises/runner_support/locator.rs"]
mod locator;
#[path = "../exercises/runner_stdio/model.rs"]
mod model;
#[path = "../exercises/runner_support/protocol.rs"]
mod protocol;
#[path = "../exercises/runner_stdio/solution.rs"]
mod solution;

fn show(label: &str, report: &model::Report, exact: bool) {
    println!("{label}: ready={} ack={} timeout={} killed={} reaped={} joined={} exact={} stdout={} stderr={}",report.ready,report.ack,report.timed_out,report.killed,report.reaped,report.joined,exact,report.stdout.len(),report.stderr.len());
}

fn stdin() -> Result<(), String> {
    let binary = locator::example_binary()?;
    let input = b"hello\0\xff";
    for (label, hook) in [
        (
            "before held-stdin",
            faults::retain_stdin as model::SendInput,
        ),
        (
            "after closed-stdin",
            solution::send_input as model::SendInput,
        ),
    ] {
        let report = model::run(&binary, "stdin", input, hook, solution::start_drains)?;
        assert!(report.ready && report.ack && report.reaped && report.joined);
        let exact =
            report.stdout == protocol::stdin_stdout(input) && report.stderr == protocol::STDIN_EOF;
        if label.starts_with("after") {
            assert!(!report.timed_out && report.status.success() && exact);
        } else {
            assert!(report.timed_out && report.killed && report.stderr.is_empty());
        }
        show(label, &report, exact);
    }
    Ok(())
}

fn dual() -> Result<(), String> {
    let binary = locator::example_binary()?;
    for (label, hook) in [
        (
            "before wait-before-drain",
            faults::wait_before_bulk_drain as model::StartDrains,
        ),
        (
            "before sequential-drain",
            faults::stdout_then_stderr as model::StartDrains,
        ),
        (
            "after parallel-drain",
            solution::start_drains as model::StartDrains,
        ),
    ] {
        let report = model::run(&binary, "dual", b"G", solution::send_input, hook)?;
        assert!(report.ready && report.ack && report.reaped && report.joined);
        let exact =
            report.stdout == protocol::dual_stdout() && report.stderr == protocol::dual_stderr();
        if label.starts_with("after") {
            assert!(!report.timed_out && report.status.success() && exact);
        } else {
            assert!(report.timed_out && report.killed && !exact);
        }
        show(label, &report, exact);
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "stdin" => stdin(),
        "dual" => dual(),
        "all" => {
            stdin()?;
            dual()
        }
        _ => Err("usage: runner_stdio_problem_chain [stdin|dual|all]".to_owned()),
    }
}

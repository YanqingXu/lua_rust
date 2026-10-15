//! Fixed lifecycle faults and supervised real fixture; never reads learner starter.
#[path = "../exercises/runner_timeout/contracts.rs"]
mod contracts;
#[path = "../exercises/runner_timeout/faults.rs"]
mod faults;
#[path = "../exercises/runner_support/locator.rs"]
mod locator;
#[path = "../exercises/runner_timeout/model.rs"]
pub mod model;
#[path = "../exercises/runner_support/protocol.rs"]
mod protocol;
#[path = "../exercises/runner_timeout/real.rs"]
pub mod real;
#[path = "../exercises/runner_timeout/real_contracts.rs"]
mod real_contracts;
#[path = "../exercises/runner_timeout/solution.rs"]
mod solution;

fn lifecycle() -> Result<(), String> {
    contracts::timeout_cleanup_always_attempts_wait_and_releases_exact_owner(solution::stop)?;
    for (label, stop) in [
        (
            "before return-only",
            faults::return_without_stopping as model::Stop,
        ),
        (
            "before kill-only",
            faults::kill_without_waiting as model::Stop,
        ),
        ("after kill+wait", solution::stop as model::Stop),
    ] {
        let report = model::run(stop, false, false)?;
        println!(
            "{label}: boundary={:?} backstop={:?} final_owned={} final_reaped={}",
            report.at_return, report.backstop_steps, report.final_owned, report.final_reaped
        );
    }
    let race = model::run(solution::stop, true, false)?;
    println!(
        "kill race: result={:?} steps={:?} owned={}",
        race.hook_result, race.at_return.steps, race.at_return.owned
    );
    let wait_error = model::run(solution::stop, false, true)?;
    println!(
        "wait error: result={:?} boundary_owned={} backstop={:?}",
        wait_error.hook_result, wait_error.at_return.owned, wait_error.backstop_steps
    );
    println!("normal poll: {:?}", model::already_reaped(solution::stop)?);
    Ok(())
}

fn real() -> Result<(), String> {
    let binary = locator::example_binary()?;
    for (label, stop) in [
        (
            "before return-only",
            faults::return_without_stopping as model::Stop,
        ),
        (
            "before kill-only",
            faults::kill_without_waiting as model::Stop,
        ),
        ("after kill+wait", solution::stop as model::Stop),
    ] {
        let report = real_contracts::stall(&binary, stop)?;
        println!("real {label}: timed_out={} ready={} alive_at_deadline={} boundary={:?} backstop={:?} stdout={:?} stderr={:?} final_owned={}", matches!(report.termination, real::Termination::TimedOut), report.ready_seen, report.alive_at_timeout, report.hook_boundary, report.backstop_steps, report.stdout, report.stderr, report.final_owned);
    }
    for report in real_contracts::normal_exits(&binary, solution::stop)? {
        if let real::Termination::Exited(status) = report.termination {
            println!(
                "real normal: code={:?} hook={:?} cleanup_status={:?} stdout={:?} stderr={:?}",
                status.code(),
                report.hook_boundary,
                report.cleanup_status,
                report.stdout,
                report.stderr
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "lifecycle" => lifecycle(),
        "real" => real(),
        "all" => {
            lifecycle()?;
            real()
        }
        _ => Err("usage: runner_timeout_problem_chain [lifecycle|real|all]".to_owned()),
    }
}

mod contracts;
mod faults;
pub mod model;
#[path = "../runner_support/protocol.rs"]
mod protocol;
pub mod real;
mod real_contracts;
mod solution;

use std::path::Path;

#[test]
fn timeout_cleanup_always_attempts_wait_and_releases_exact_owner() {
    contracts::timeout_cleanup_always_attempts_wait_and_releases_exact_owner(solution::stop)
        .unwrap();
}

#[test]
fn faulty_timeout_return_leaves_running_owner_at_hook_boundary() {
    let report = model::run(faults::return_without_stopping, false, false).unwrap();
    assert_eq!(report.at_return.steps, Vec::<&str>::new());
    assert!(report.at_return.owned);
    assert_eq!(report.at_return.alive_probe, Some(true));
    assert!(!report.at_return.wait_succeeded);
    assert_eq!(report.backstop_steps, ["kill", "wait"]);
    assert!(!report.final_owned && report.final_reaped);
}

#[test]
fn faulty_kill_without_wait_has_not_released_the_owner() {
    let report = model::run(faults::kill_without_waiting, false, false).unwrap();
    assert_eq!(report.at_return.steps, ["kill"]);
    assert!(report.at_return.owned && !report.at_return.wait_succeeded);
    assert_eq!(report.at_return.alive_probe, Some(false));
    assert_eq!(report.backstop_steps, ["kill", "wait"]);
    assert!(!report.final_owned && report.final_reaped);
}

#[test]
fn already_reaped_status_needs_no_second_wait_and_is_not_timeout() {
    let boundary = model::already_reaped(solution::stop).unwrap();
    assert_eq!(boundary.steps, ["try_wait Some"]);
    assert!(!boundary.owned && !boundary.wait_succeeded);
    assert_eq!(
        real_contracts::normal_exits(
            Path::new(env!("CARGO_BIN_EXE_runner_fixture")),
            solution::stop
        )
        .unwrap()
        .len(),
        2
    );
}

#[test]
fn real_ready_child_is_alive_then_killed_waited_and_preserves_partial_bytes() {
    let captured = real_contracts::stall(
        Path::new(env!("CARGO_BIN_EXE_runner_fixture")),
        solution::stop,
    )
    .unwrap();
    let boundary = captured.hook_boundary.unwrap();
    assert_eq!(boundary.steps, ["kill", "wait"]);
    assert!(!boundary.owned && boundary.wait_succeeded);
    assert!(captured.backstop_steps.is_empty() && captured.cleanup_error.is_none());
}

#[test]
fn real_fixed_faults_are_observed_before_backstop_reaps_exact_child() {
    for (stop, steps, alive) in [
        (
            faults::return_without_stopping as model::Stop,
            vec![],
            Some(true),
        ),
        (
            faults::kill_without_waiting as model::Stop,
            vec!["kill"],
            None,
        ),
    ] {
        let captured =
            real_contracts::stall(Path::new(env!("CARGO_BIN_EXE_runner_fixture")), stop).unwrap();
        let boundary = captured.hook_boundary.unwrap();
        assert_eq!(boundary.steps, steps);
        assert!(boundary.owned && !boundary.wait_succeeded);
        assert_eq!(boundary.alive_probe, alive);
        assert_eq!(captured.backstop_steps, ["kill", "wait"]);
    }
}

use super::model::{self, Stop};
use std::fmt::Debug;

pub fn equal<T: Debug + PartialEq>(label: &str, actual: T, expected: T) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label}: expected {expected:?}, got {actual:?}"))
    }
}

// ANCHOR: timeout_contract
pub fn timeout_cleanup_always_attempts_wait_and_releases_exact_owner(
    stop: Stop,
) -> Result<(), String> {
    for race in [false, true] {
        let report = model::run(stop, race, false)?;
        equal(
            "kill and wait even on kill race",
            report.at_return.steps,
            vec!["kill", "wait"],
        )?;
        equal("hook completes cleanup", report.hook_result, Ok(()))?;
        equal(
            "owner released at hook return",
            report.at_return.owned,
            false,
        )?;
        equal(
            "wait completed at hook return",
            report.at_return.wait_succeeded,
            true,
        )?;
        equal("no hidden backstop repair", report.backstop_steps, vec![])?;
        equal(
            "final guard state",
            (report.final_owned, report.final_reaped),
            (false, true),
        )?;
    }
    let failed = model::run(stop, false, true)?;
    equal(
        "wait failure is not swallowed",
        failed.hook_result,
        Err("wait after kill (Ok(())): injected wait failure".to_owned()),
    )?;
    equal(
        "wait still attempted",
        failed.at_return.steps,
        vec!["kill", "wait"],
    )?;
    equal(
        "failed wait retains owner for backstop",
        failed.at_return.owned,
        true,
    )?;
    equal(
        "backstop retries exact owner",
        failed.backstop_steps,
        vec!["kill", "wait"],
    )?;
    equal(
        "failure path eventually reaped",
        (failed.final_owned, failed.final_reaped),
        (false, true),
    )?;
    Ok(())
}
// ANCHOR_END: timeout_contract

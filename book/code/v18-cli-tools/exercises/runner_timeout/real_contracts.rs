use super::contracts::equal;
use super::model::Stop;
use super::real::{capture, Capture, RunSpec, Termination};
use crate::protocol;
use std::path::Path;
use std::time::Duration;

pub fn stall(binary: &Path, stop: Stop) -> Result<Capture, String> {
    let args = vec!["stall".to_owned()];
    let capture = capture(
        RunSpec {
            binary,
            args: &args,
            stdin: b"",
            hold_stdin: true,
            timeout: Duration::from_millis(100),
            ready_prefix: Some(b"READY stall\n"),
        },
        stop,
    )?;
    equal(
        "timeout has its own classification",
        matches!(capture.termination, Termination::TimedOut),
        true,
    )?;
    equal("real READY observed", capture.ready_seen, true)?;
    equal("try_wait None at deadline", capture.alive_at_timeout, true)?;
    equal(
        "partial stdout exact bytes",
        capture.stdout.clone(),
        protocol::STALL_STDOUT.to_vec(),
    )?;
    equal(
        "partial stderr exact bytes",
        capture.stderr.clone(),
        protocol::STALL_STDERR.to_vec(),
    )?;
    equal(
        "no process owner remains after supervised cleanup",
        capture.final_owned,
        false,
    )?;
    equal(
        "OS wait status collected",
        capture.cleanup_status.is_some(),
        true,
    )?;
    Ok(capture)
}

pub fn normal_exits(binary: &Path, stop: Stop) -> Result<Vec<Capture>, String> {
    let mut reports = Vec::new();
    for code in [0, 7] {
        let args = vec!["exit".to_owned(), code.to_string()];
        let capture = capture(
            RunSpec {
                binary,
                args: &args,
                stdin: b"",
                hold_stdin: false,
                timeout: Duration::from_secs(2),
                ready_prefix: None,
            },
            stop,
        )?;
        match capture.termination {
            Termination::Exited(status) => {
                equal("normal exit is not a timeout", status.code(), Some(code))?
            }
            Termination::TimedOut => return Err("normal exit classified as timeout".to_owned()),
        }
        equal(
            "normal stdout",
            capture.stdout.clone(),
            protocol::EXIT_STDOUT.to_vec(),
        )?;
        equal(
            "normal stderr",
            capture.stderr.clone(),
            protocol::EXIT_STDERR.to_vec(),
        )?;
        equal(
            "timeout hook not invoked",
            capture.hook_boundary.is_none(),
            true,
        )?;
        equal(
            "try_wait Some needs no repeated wait",
            capture.backstop_steps.clone(),
            vec![],
        )?;
        equal(
            "no cleanup kill status on normal completion",
            capture.cleanup_status.is_none(),
            true,
        )?;
        equal("normal child owner released", capture.final_owned, false)?;
        reports.push(capture);
    }
    Ok(reports)
}

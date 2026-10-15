use crate::contracts::{require, verify};
use crate::model::{Capture, Evidence, Expected, Termination};
use crate::storage::{read_archive, Archive};
use crate::{protocol, timeout};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub fn run_id() -> Result<String, String> {
    Ok(format!(
        "run-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn capture(binary: &Path, args: &[String], stdin: &[u8], stall: bool) -> Result<Capture, String> {
    let observed = timeout::real::capture(
        timeout::real::RunSpec {
            binary,
            args,
            stdin,
            hold_stdin: stall,
            timeout: if stall {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(3)
            },
            ready_prefix: stall.then_some(&b"READY stall\n"[..]),
        },
        timeout::solution::stop,
    )?;
    let termination = match observed.termination {
        timeout::real::Termination::Exited(status) => Termination::Exited {
            code: status.code(),
            success: status.success(),
            status_debug: format!("{status:?}"),
        },
        timeout::real::Termination::TimedOut => Termination::TimedOut {
            cleanup_code: observed
                .cleanup_status
                .as_ref()
                .and_then(|status| status.code()),
            cleanup_success: observed
                .cleanup_status
                .as_ref()
                .map(|status| status.success()),
            cleanup_status_debug: observed
                .cleanup_status
                .as_ref()
                .map(|status| format!("{status:?}")),
            ready_seen: observed.ready_seen,
            alive_at_timeout: observed.alive_at_timeout,
            reaped: !observed.final_owned && observed.cleanup_status.is_some(),
            cleanup_error: observed.cleanup_error,
        },
    };
    Ok(Capture {
        stdout: observed.stdout,
        stderr: observed.stderr,
        elapsed: observed.elapsed,
        termination,
    })
}

pub fn fixtures(binary: &Path) -> Result<[Evidence; 3], String> {
    let binary = binary.canonicalize().map_err(|error| error.to_string())?;
    let version = capture(&binary, &["version".into()], b"", false)?;
    require(
        version.stdout == protocol::VERSION
            && version.stderr.is_empty()
            && matches!(
                version.termination,
                Termination::Exited { code: Some(0), .. }
            ),
        "fixture version process mismatch",
    )?;
    let mut cases = Vec::new();
    for code in [7, 9] {
        let argv = vec!["exit".into(), code.to_string()];
        // exit 模式不读 stdin；空请求避免子进程先退出造成 BrokenPipe 竞态。
        let stdin = Vec::new();
        let capture = capture(&binary, &argv, &stdin, false)?;
        require(
            capture.stdout == protocol::EXIT_STDOUT && capture.stderr == protocol::EXIT_STDERR,
            "real raw output mismatch",
        )?;
        require(
            matches!(capture.termination,Termination::Exited {code:Some(actual),success:false,..} if actual==code),
            "real nonzero exit mismatch",
        )?;
        cases.push(Evidence {
            run_id: run_id()?,
            case_id: "same-case".into(),
            executable: binary.clone(),
            version: version.stdout.clone(),
            argv,
            stdin,
            capture,
            expected: Expected {
                exit_code: if code == 7 { 0 } else { 9 },
                stdout: protocol::EXIT_STDOUT.to_vec(),
                stderr: protocol::EXIT_STDERR.to_vec(),
            },
        });
    }
    let argv = vec!["stall".into()];
    let stdin = b"submitted but EOF deliberately held\0\xff\n".to_vec();
    let capture = capture(&binary, &argv, &stdin, true)?;
    require(
        capture.stdout == protocol::STALL_STDOUT && capture.stderr == protocol::STALL_STDERR,
        "timeout partial output mismatch",
    )?;
    require(
        matches!(
            &capture.termination,
            Termination::TimedOut {
                ready_seen: true,
                alive_at_timeout: true,
                reaped: true,
                cleanup_error: None,
                ..
            }
        ),
        "timeout cleanup facts mismatch",
    )?;
    cases.push(Evidence {
        run_id: run_id()?,
        case_id: "timeout".into(),
        executable: binary,
        version: version.stdout,
        argv,
        stdin,
        capture,
        expected: Expected {
            exit_code: 0,
            stdout: protocol::STALL_STDOUT.to_vec(),
            stderr: protocol::STALL_STDERR.to_vec(),
        },
    });
    cases
        .try_into()
        .map_err(|_| "expected three fixture captures".into())
}

pub fn archive_fixtures(
    root: &Path,
    binary: &Path,
    archive: Archive,
) -> Result<Vec<PathBuf>, String> {
    let cases = fixtures(binary)?;
    let mut paths = Vec::new();
    for case in &cases {
        let path = archive(root, case)?;
        verify(&path, case)?;
        paths.push(path);
    }
    require(
        paths[0] != paths[1],
        "same case across runs overwrote old evidence",
    )?;
    for (index, passed) in [(0, false), (1, true), (2, false)] {
        require(
            read_archive(&paths[index])?.metadata["verdict.passed"] == passed.to_string(),
            "real semantic verdict mismatch",
        )?;
    }
    let timeout = read_archive(&paths[2])?;
    require(
        timeout.metadata["termination.kind"] == "timed-out"
            && !timeout.metadata.contains_key("exit.code"),
        "timeout mislabeled as ordinary exit",
    )?;
    require(
        timeout.metadata["cleanup.reaped"] == "true",
        "cleanup wait evidence missing",
    )?;
    // 再读第一次，确认重复case没有悄悄改掉旧metadata/bytes。
    verify(&paths[0], &cases[0])?;
    Ok(paths)
}

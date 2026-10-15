use crate::model::{self, Capture, Evidence, Expected, Termination};
use crate::storage::{self, Archive};
use crate::temp::TempDirectory;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub fn sample(run: &str) -> Evidence {
    Evidence {
        run_id: run.into(),
        case_id: "binary".into(),
        executable: std::env::current_exe().unwrap(),
        version: b"synthetic capture v1\n".to_vec(),
        argv: vec!["exit".into(), "7".into()],
        stdin: b"input\0\xff\n".to_vec(),
        capture: Capture {
            stdout: b"OUT\0\xff\n".to_vec(),
            stderr: b"ERR\0\xfe\n".to_vec(),
            elapsed: Duration::from_millis(17),
            termination: Termination::Exited {
                code: Some(7),
                success: false,
                status_debug: "synthetic exit 7".into(),
            },
        },
        expected: Expected {
            exit_code: 7,
            stdout: b"EXP\0\0\n".to_vec(),
            stderr: b"BAD\0\0\n".to_vec(),
        },
    }
}

pub fn verify(directory: &Path, evidence: &Evidence) -> Result<(), String> {
    let saved = storage::read_archive(directory)?;
    require(
        saved.stdin == evidence.stdin,
        "saved stdin differs from requested raw bytes",
    )?;
    require(
        saved.stdout == evidence.capture.stdout,
        "actual stdout was replaced/lost",
    )?;
    require(
        saved.stderr == evidence.capture.stderr,
        "actual stderr was replaced/lost",
    )?;
    require(saved.version == evidence.version, "version bytes changed")?;
    require(
        saved.metadata == storage::fields(&model::metadata(evidence)?)?,
        "metadata differs from the same observed capture",
    )?;
    require(
        saved.manifest["run_id"] == evidence.run_id
            && saved.manifest["case_id"] == evidence.case_id,
        "manifest belongs to another run/case",
    )?;
    Ok(())
}

// ANCHOR: runner_evidence_contract
pub fn archives_observed_bytes_and_same_run_metadata(archive: Archive) -> Result<(), String> {
    let temp = TempDirectory::new("evidence-contract")?;
    let result = (|| {
        let root = temp.path("archives");
        let first = sample("run-first");
        let first_path = archive(&root, &first)?;
        verify(&first_path, &first)?;
        let saved = storage::read_archive(&first_path)?;
        require(
            saved.metadata["termination.kind"] == "exited" && saved.metadata["exit.code"] == "7",
            "normal exit fact lost",
        )?;
        require(
            saved.metadata["elapsed.ns"] == "17000000",
            "elapsed fact lost",
        )?;
        require(
            saved.metadata["argv.0.utf8.hex"] == "65786974",
            "argv fact lost",
        )?;
        require(
            saved.metadata["verdict.passed"] == "false",
            "failed bytes must not become a green report",
        )?;
        repeat_keeps_first(archive, &root, &first, &first_path)
    })();
    temp.finish(result)
}
// ANCHOR_END: runner_evidence_contract

// ANCHOR: runner_evidence_repeat
fn repeat_keeps_first(
    archive: Archive,
    root: &Path,
    first: &Evidence,
    first_path: &Path,
) -> Result<(), String> {
    let before = fs::read(first_path.join("metadata.tsv")).map_err(|error| error.to_string())?;
    let mut second = sample("run-second"); // 同名case，另一run。
    second.stdin = b"second\n".to_vec();
    second.argv[1] = "9".into();
    second.capture.stdout[0] = b'Z';
    second.capture.termination = Termination::Exited {
        code: Some(9),
        success: false,
        status_debug: "synthetic exit 9".into(),
    };
    second.expected = Expected {
        exit_code: 9,
        stdout: second.capture.stdout.clone(),
        stderr: second.capture.stderr.clone(),
    };
    let second_path = archive(root, &second)?;
    require(
        first_path != second_path,
        "repeat overwrote the first directory",
    )?;
    verify(&second_path, &second)?;
    verify(first_path, first)?;
    require(
        storage::read_archive(&second_path)?.metadata["verdict.passed"] == "true",
        "declared nonzero expected exit can pass",
    )?;
    require(
        archive(root, first).is_err(),
        "same run/case must refuse overwrite",
    )?;
    require(
        fs::read(first_path.join("metadata.tsv")).map_err(|error| error.to_string())? == before,
        "collision changed old facts",
    )?;
    let mut invalid = first.clone();
    invalid.run_id = "../escape".into();
    require(
        archive(root, &invalid).is_err(),
        "invalid run component accepted",
    )?;
    Ok(())
}
// ANCHOR_END: runner_evidence_repeat

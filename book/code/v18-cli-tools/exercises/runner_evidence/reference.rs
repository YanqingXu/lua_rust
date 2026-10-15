mod contracts;
mod faults;
pub mod model;
#[path = "../runner_support/protocol.rs"]
pub mod protocol;
pub mod real;
mod solution;
pub mod storage;
#[path = "../support/temp.rs"]
pub mod temp;
pub mod timeout;

use std::fs;
use std::path::Path;

#[test]
fn archives_observed_bytes_and_same_run_metadata() {
    contracts::archives_observed_bytes_and_same_run_metadata(solution::archive).unwrap();
}

#[test]
fn faulty_expected_output_overwrites_the_observed_fact() {
    let temp = temp::TempDirectory::new("evidence-expected").unwrap();
    let result = (|| {
        let evidence = contracts::sample("run-copy");
        let path = faults::expected_as_actual(&temp.path("archives"), &evidence)?;
        let saved = storage::read_archive(&path)?;
        contracts::require(
            saved.stdout == evidence.expected.stdout && saved.stdout != evidence.capture.stdout,
            "expected-copy fault did not corrupt actual stdout",
        )?;
        contracts::require(
            saved.metadata["verdict.passed"] == "false",
            "fixed metadata must not hide original failure",
        )?;
        contracts::require(
            contracts::verify(&path, &evidence).is_err(),
            "full contract accepted corrupted bytes",
        )
    })();
    temp.finish(result).unwrap();
}

#[test]
fn faulty_latest_path_discards_a_previous_same_name_run() {
    let temp = temp::TempDirectory::new("evidence-latest").unwrap();
    let result = (|| {
        let root = temp.path("archives");
        let first = contracts::sample("run-first");
        let path = faults::overwrite_latest(&root, &first)?;
        let before = fs::read(path.join("metadata.tsv")).map_err(|error| error.to_string())?;
        let second = contracts::sample("run-second");
        let next = faults::overwrite_latest(&root, &second)?;
        contracts::require(path == next, "fixed latest path unexpectedly isolated runs")?;
        let after = fs::read(path.join("metadata.tsv")).map_err(|error| error.to_string())?;
        contracts::require(
            before != after && storage::fields(&after)?["run_id"] == "run-second",
            "first run metadata was not overwritten",
        )
    })();
    temp.finish(result).unwrap();
}

#[test]
fn io_failure_returns_error_and_keeps_partial_directory_incomplete() {
    let temp = temp::TempDirectory::new("evidence-io").unwrap();
    let result = (|| {
        let evidence = contracts::sample("run-partial");
        let writer = storage::CaseWriter::begin(&temp.path("archives"), &evidence)?;
        writer.write_new("stdin.bin", &evidence.stdin)?;
        writer.write_new("stdout.bin", &evidence.capture.stdout)?;
        fs::create_dir(writer.directory.join("stderr.bin")).map_err(|error| error.to_string())?;
        contracts::require(
            writer
                .write_new("stderr.bin", &evidence.capture.stderr)
                .is_err(),
            "write collision must return Err",
        )?;
        contracts::require(
            writer.directory.join("INCOMPLETE").is_file()
                && !writer.directory.join("manifest.tsv").exists(),
            "partial directory was marked complete",
        )?;
        contracts::require(
            storage::read_archive(&writer.directory).is_err(),
            "reader accepted partial archive",
        )?;
        let file_root = temp.path("not-directory");
        fs::write(&file_root, b"keep me").map_err(|error| error.to_string())?;
        contracts::require(
            solution::archive(&file_root, &evidence).is_err(),
            "archive hid root I/O error",
        )?;
        contracts::require(
            fs::read(file_root).map_err(|error| error.to_string())? == b"keep me",
            "root failure changed existing file",
        )
    })();
    temp.finish(result).unwrap();
}

#[test]
fn timeout_cleanup_code_zero_is_not_a_normal_successful_exit() {
    let temp = temp::TempDirectory::new("evidence-timeout-kind").unwrap();
    let result = (|| {
        let mut evidence = contracts::sample("run-timeout-model");
        evidence.expected = model::Expected {
            exit_code: 0,
            stdout: evidence.capture.stdout.clone(),
            stderr: evidence.capture.stderr.clone(),
        };
        evidence.capture.termination = model::Termination::TimedOut {
            cleanup_code: Some(0),
            cleanup_success: Some(true),
            ready_seen: true,
            alive_at_timeout: true,
            reaped: true,
            cleanup_error: None,
            cleanup_status_debug: Some("synthetic cleanup status 0".into()),
        };
        let path = solution::archive(&temp.path("archives"), &evidence)?;
        contracts::verify(&path, &evidence)?;
        let saved = storage::read_archive(&path)?;
        contracts::require(
            saved.metadata["termination.kind"] == "timed-out"
                && saved.metadata["cleanup.code"] == "0"
                && !saved.metadata.contains_key("exit.code"),
            "cleanup status conflated with ordinary exit",
        )?;
        contracts::require(
            saved.metadata["verdict.stdout_matches"] == "true"
                && saved.metadata["verdict.stderr_matches"] == "true"
                && saved.metadata["verdict.passed"] == "false",
            "timeout became green because streams matched",
        )
    })();
    temp.finish(result).unwrap();
}

#[test]
fn reader_rejects_truncated_manifest_or_wrong_run_case_identity() {
    let temp = temp::TempDirectory::new("evidence-reader").unwrap();
    let result = (|| {
        let evidence = contracts::sample("run-reader");
        let path = solution::archive(&temp.path("archives"), &evidence)?;
        let manifest = path.join("manifest.tsv");
        let original = fs::read(&manifest).map_err(|error| error.to_string())?;
        fs::write(&manifest, b"schema\trunner-evidence-v1\nstate\tcomple")
            .map_err(|error| error.to_string())?;
        contracts::require(
            storage::read_archive(&path).is_err(),
            "reader accepted truncated completion marker",
        )?;
        fs::write(
            &manifest,
            String::from_utf8(original)
                .unwrap()
                .replace("case_id\tbinary", "case_id\tother"),
        )
        .map_err(|error| error.to_string())?;
        contracts::require(
            storage::read_archive(&path).is_err(),
            "reader accepted mismatched case metadata",
        )
    })();
    temp.finish(result).unwrap();
}

#[test]
fn real_children_archive_raw_bytes_repeat_and_timeout_without_greenwashing() {
    let temp = temp::TempDirectory::new("evidence-real").unwrap();
    let result = real::archive_fixtures(
        &temp.path("archives"),
        Path::new(env!("CARGO_BIN_EXE_runner_fixture")),
        solution::archive,
    );
    assert_eq!(temp.finish(result).unwrap().len(), 3);
}

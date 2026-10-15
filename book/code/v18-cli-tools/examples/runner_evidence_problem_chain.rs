#[path = "../exercises/runner_evidence/contracts.rs"]
mod contracts;
#[path = "../exercises/runner_evidence/faults.rs"]
mod faults;
#[path = "../exercises/runner_support/locator.rs"]
pub mod locator;
#[path = "../exercises/runner_evidence/model.rs"]
pub mod model;
#[path = "../exercises/runner_support/protocol.rs"]
pub mod protocol;
#[path = "../exercises/runner_evidence/real.rs"]
pub mod real;
#[path = "../exercises/runner_evidence/solution.rs"]
mod solution;
#[path = "../exercises/runner_evidence/storage.rs"]
pub mod storage;
#[path = "../exercises/support/temp.rs"]
pub mod temp;
#[path = "../exercises/runner_evidence/timeout.rs"]
pub mod timeout;

use std::fs;
use std::path::Path;

fn fixed(root: &Path) -> Result<(), String> {
    let evidence = contracts::sample("run-expected-copy");
    let path = faults::expected_as_actual(root, &evidence)?;
    let saved = storage::read_archive(&path)?;
    println!(
        "before expected-copy: saved_expected={} observed_preserved={} verdict={}",
        saved.stdout == evidence.expected.stdout,
        saved.stdout == evidence.capture.stdout,
        saved.metadata["verdict.passed"]
    );
    let first = contracts::sample("run-latest-first");
    let first_path = faults::overwrite_latest(root, &first)?;
    let before = fs::read(first_path.join("metadata.tsv")).map_err(|error| error.to_string())?;
    let second = contracts::sample("run-latest-second");
    let second_path = faults::overwrite_latest(root, &second)?;
    let after = fs::read(second_path.join("metadata.tsv")).map_err(|error| error.to_string())?;
    println!(
        "before latest: same_path={} first_metadata_lost={}",
        first_path == second_path,
        before != after
    );
    contracts::archives_observed_bytes_and_same_run_metadata(solution::archive)?;
    let mut correct = contracts::sample("run-correct");
    correct.case_id = "binary".into();
    let path = solution::archive(root, &correct)?;
    contracts::verify(&path, &correct)?;
    let saved = storage::read_archive(&path)?;
    println!(
        "after observed: stdout_hex={} stderr_hex={} exit={} passed={}",
        model::hex(&saved.stdout),
        model::hex(&saved.stderr),
        saved.metadata["exit.code"],
        saved.metadata["verdict.passed"]
    );
    let before = fs::read(path.join("metadata.tsv")).map_err(|error| error.to_string())?;
    let second = contracts::sample("run-correct-repeat");
    let repeated = solution::archive(root, &second)?;
    contracts::verify(&path, &correct)?;
    contracts::verify(&repeated, &second)?;
    let distinct = path != repeated;
    let same_case =
        storage::read_archive(&repeated)?.metadata["case_id"] == saved.metadata["case_id"];
    let rejected = solution::archive(root, &correct).is_err();
    let unchanged =
        fs::read(path.join("metadata.tsv")).map_err(|error| error.to_string())? == before;
    contracts::require(
        distinct && same_case && rejected && unchanged,
        "archive isolation regression",
    )?;
    println!("after isolation: distinct_runs={distinct} same_case={same_case} collision_rejected={rejected}");
    Ok(())
}

fn actual(root: &Path, export: bool) -> Result<(), String> {
    let binary = locator::example_binary()?;
    let paths = real::archive_fixtures(root, &binary, solution::archive)?;
    let cases = paths
        .iter()
        .map(|path| storage::read_archive(path))
        .collect::<Result<Vec<_>, _>>()?;
    println!("real archive: cases={} verdicts=[{},{},{}] normal_codes=[{},{}] timeout_kind={} timeout_reaped={}",cases.len(),cases[0].metadata["verdict.passed"],cases[1].metadata["verdict.passed"],cases[2].metadata["verdict.passed"],cases[0].metadata["exit.code"],cases[1].metadata["exit.code"],cases[2].metadata["termination.kind"],cases[2].metadata["cleanup.reaped"]);
    if export {
        for path in paths {
            println!("exported {}", path.join("manifest.tsv").display());
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "all".into());
    if mode == "export" {
        let root = args
            .next()
            .ok_or("usage: runner_evidence_problem_chain export <directory>")?;
        return actual(Path::new(&root), true);
    }
    let directory = temp::TempDirectory::new("evidence-cli")?;
    let root = directory.path("archives");
    let result = match mode.as_str() {
        "faults" => fixed(&root),
        "real" => actual(&root, false),
        "all" => fixed(&root).and_then(|()| actual(&root, false)),
        _ => {
            Err("usage: runner_evidence_problem_chain [faults|real|all|export <directory>]".into())
        }
    };
    directory.finish(result)
}

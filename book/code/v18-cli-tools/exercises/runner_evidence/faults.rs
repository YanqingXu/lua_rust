use crate::model::{metadata, Evidence};
use crate::storage::CaseWriter;
use std::fs;
use std::path::{Path, PathBuf};

// ANCHOR: runner_evidence_expected_fault
pub fn expected_as_actual(root: &Path, evidence: &Evidence) -> Result<PathBuf, String> {
    let writer = CaseWriter::begin(root, evidence)?;
    writer.write_new("stdin.bin", &evidence.stdin)?;
    writer.write_new("stdout.bin", &evidence.expected.stdout)?;
    writer.write_new("stderr.bin", &evidence.expected.stderr)?;
    writer.complete(evidence)
}
// ANCHOR_END: runner_evidence_expected_fault

// ANCHOR: runner_evidence_latest_fault
pub fn overwrite_latest(root: &Path, evidence: &Evidence) -> Result<PathBuf, String> {
    let latest = root.join("latest");
    fs::create_dir_all(&latest).map_err(|error| error.to_string())?;
    fs::write(latest.join("stdout.bin"), &evidence.capture.stdout)
        .map_err(|error| error.to_string())?;
    fs::write(latest.join("metadata.tsv"), metadata(evidence)?)
        .map_err(|error| error.to_string())?;
    Ok(latest)
}
// ANCHOR_END: runner_evidence_latest_fault

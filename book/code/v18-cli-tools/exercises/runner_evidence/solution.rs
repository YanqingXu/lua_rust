use crate::model::Evidence;
use crate::storage::CaseWriter;
use std::path::{Path, PathBuf};

// ANCHOR: runner_evidence_solution
pub fn archive(root: &Path, evidence: &Evidence) -> Result<PathBuf, String> {
    let writer = CaseWriter::begin(root, evidence)?;
    writer.write_new("stdin.bin", &evidence.stdin)?;
    writer.write_new("stdout.bin", &evidence.capture.stdout)?;
    writer.write_new("stderr.bin", &evidence.capture.stderr)?;
    writer.complete(evidence)
}
// ANCHOR_END: runner_evidence_solution

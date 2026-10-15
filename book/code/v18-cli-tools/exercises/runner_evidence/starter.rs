use crate::model::Evidence;
use crate::storage::CaseWriter;
use std::path::{Path, PathBuf};

// ANCHOR: runner_evidence_starter
pub fn archive(root: &Path, evidence: &Evidence) -> Result<PathBuf, String> {
    let writer = CaseWriter::begin(root, evidence)?;
    // TODO：pass bool不足以复查；分别保存本次真实stdin/stdout/stderr原bytes。
    writer.write_new(
        "summary.txt",
        evidence.verdict().passed.to_string().as_bytes(),
    )?;
    writer.complete(evidence)
}
// ANCHOR_END: runner_evidence_starter

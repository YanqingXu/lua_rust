use crate::model::{metadata, Evidence};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub type Archive = fn(&Path, &Evidence) -> Result<PathBuf, String>;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub struct CaseWriter {
    pub directory: PathBuf,
}

// ANCHOR: runner_evidence_begin
impl CaseWriter {
    pub fn begin(root: &Path, evidence: &Evidence) -> Result<Self, String> {
        if !valid_id(&evidence.run_id) || !valid_id(&evidence.case_id) {
            return Err("run_id/case_id must be 1..64 ASCII letters/digits/_/-".into());
        }
        metadata(evidence)?; // 在创建目录前验证 executable/metadata 输入。
        fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let run = root.join(&evidence.run_id);
        match fs::create_dir(&run) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let run = run.canonicalize().map_err(|error| error.to_string())?;
        if run.parent() != Some(root.as_path())
            || run.file_name() != Some(std::ffi::OsStr::new(&evidence.run_id))
        {
            return Err("run directory escaped archive root".into());
        }
        let directory = run.join(&evidence.case_id);
        fs::create_dir(&directory).map_err(|error| format!("exclusive case create: {error}"))?;
        let writer = Self { directory };
        writer.write_new("INCOMPLETE", b"not a complete archive\n")?;
        Ok(writer)
    }
    // ANCHOR_END: runner_evidence_begin

    pub fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if Path::new(name).components().count() != 1
            || !matches!(
                name,
                "INCOMPLETE"
                    | "stdin.bin"
                    | "stdout.bin"
                    | "stderr.bin"
                    | "binary-version.bin"
                    | "metadata.tsv"
                    | "manifest.tsv"
                    | "summary.txt"
            )
        {
            return Err("unsupported archive filename".into());
        }
        let path = self.directory.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("create {name}: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("write {name}: {error}"))
    }

    // ANCHOR: runner_evidence_complete
    pub fn complete(self, evidence: &Evidence) -> Result<PathBuf, String> {
        for name in ["stdin.bin", "stdout.bin", "stderr.bin"] {
            if !self.directory.join(name).is_file() {
                return Err(format!("missing captured payload {name}"));
            }
        }
        self.write_new("binary-version.bin", &evidence.version)?;
        self.write_new("metadata.tsv", &metadata(evidence)?)?;
        let manifest=format!("schema\trunner-evidence-v1\nrun_id\t{}\ncase_id\t{}\nstdin\tstdin.bin\nstdout\tstdout.bin\nstderr\tstderr.bin\nversion\tbinary-version.bin\nmetadata\tmetadata.tsv\nstate\tcomplete\n",evidence.run_id,evidence.case_id);
        self.write_new("manifest.tsv", manifest.as_bytes())?; // 最后创建的文件。
        fs::remove_file(self.directory.join("INCOMPLETE"))
            .map_err(|error| format!("clear incomplete marker: {error}"))?;
        Ok(self.directory)
    }
}
// ANCHOR_END: runner_evidence_complete

pub fn fields(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    if !text.ends_with('\n') {
        return Err("truncated metadata/manifest".into());
    }
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('\t').ok_or("invalid metadata row")?;
        if fields.insert(key.into(), value.into()).is_some() {
            return Err("duplicate metadata key".into());
        }
    }
    Ok(fields)
}

#[derive(Debug)]
pub struct Readback {
    pub stdin: Vec<u8>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub version: Vec<u8>,
    pub metadata: BTreeMap<String, String>,
    pub manifest: BTreeMap<String, String>,
}

pub fn read_archive(directory: &Path) -> Result<Readback, String> {
    if directory.join("INCOMPLETE").exists() {
        return Err("archive is INCOMPLETE".into());
    }
    let read = |name: &str| {
        fs::read(directory.join(name)).map_err(|error| format!("read {name}: {error}"))
    };
    let manifest = fields(&read("manifest.tsv")?)?;
    for (key, expected) in [
        ("schema", "runner-evidence-v1"),
        ("state", "complete"),
        ("stdin", "stdin.bin"),
        ("stdout", "stdout.bin"),
        ("stderr", "stderr.bin"),
        ("version", "binary-version.bin"),
        ("metadata", "metadata.tsv"),
    ] {
        if manifest.get(key).map(String::as_str) != Some(expected) {
            return Err(format!("invalid manifest {key}"));
        }
    }
    let metadata = fields(&read("metadata.tsv")?)?;
    for key in ["schema", "run_id", "case_id"] {
        if metadata.get(key) != manifest.get(key) {
            return Err(format!("metadata/manifest mismatch: {key}"));
        }
    }
    if directory.file_name().and_then(|name| name.to_str())
        != metadata.get("case_id").map(String::as_str)
        || directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != metadata.get("run_id").map(String::as_str)
    {
        return Err("directory/run/case identity mismatch".into());
    }
    let stdin = read("stdin.bin")?;
    let stdout = read("stdout.bin")?;
    let stderr = read("stderr.bin")?;
    for (name, bytes) in [("stdin", &stdin), ("stdout", &stdout), ("stderr", &stderr)] {
        if metadata.get(&format!("{name}.bytes")) != Some(&bytes.len().to_string()) {
            return Err(format!("payload length mismatch: {name}"));
        }
    }
    let version = read("binary-version.bin")?;
    if metadata.get("binary.version.bytes") != Some(&version.len().to_string()) {
        return Err("version length mismatch".into());
    }
    Ok(Readback {
        stdin,
        stdout,
        stderr,
        version,
        metadata,
        manifest,
    })
}

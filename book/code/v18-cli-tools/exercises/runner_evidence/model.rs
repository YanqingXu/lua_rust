use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum Termination {
    Exited {
        code: Option<i32>,
        success: bool,
        status_debug: String,
    },
    TimedOut {
        cleanup_code: Option<i32>,
        cleanup_success: Option<bool>,
        ready_seen: bool,
        alive_at_timeout: bool,
        reaped: bool,
        cleanup_error: Option<String>,
        cleanup_status_debug: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
    pub termination: Termination,
}

#[derive(Clone, Debug)]
pub struct Expected {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Evidence {
    pub run_id: String,
    pub case_id: String,
    pub executable: PathBuf,
    pub version: Vec<u8>,
    pub argv: Vec<String>,
    pub stdin: Vec<u8>,
    pub capture: Capture,
    pub expected: Expected,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Verdict {
    pub stdout_matches: bool,
    pub stderr_matches: bool,
    pub exit_matches: bool,
    pub passed: bool,
}

// ANCHOR: runner_evidence_verdict
impl Evidence {
    pub fn verdict(&self) -> Verdict {
        let stdout_matches = self.capture.stdout == self.expected.stdout;
        let stderr_matches = self.capture.stderr == self.expected.stderr;
        let exit_matches = matches!(self.capture.termination,
            Termination::Exited { code: Some(code), .. } if code == self.expected.exit_code);
        Verdict {
            stdout_matches,
            stderr_matches,
            exit_matches,
            passed: stdout_matches && stderr_matches && exit_matches,
        }
    }
}
// ANCHOR_END: runner_evidence_verdict

pub fn hex(bytes: &[u8]) -> String {
    let mut text = String::new();
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub fn metadata(evidence: &Evidence) -> Result<Vec<u8>, String> {
    if !evidence.executable.is_absolute() {
        return Err("executable path must be absolute".into());
    }
    let executable = evidence
        .executable
        .to_str()
        .ok_or("this fixture metadata requires a UTF-8 executable path")?;
    let mut fields = BTreeMap::<String, String>::new();
    let mut put = |key: &str, value: String| {
        fields.insert(key.into(), value);
    };
    put("schema", "runner-evidence-v1".into());
    put("run_id", evidence.run_id.clone());
    put("case_id", evidence.case_id.clone());
    put("executable.utf8.hex", hex(executable.as_bytes()));
    put("binary.version.file", "binary-version.bin".into());
    put("binary.version.bytes", evidence.version.len().to_string());
    put("argv.count", evidence.argv.len().to_string());
    for (index, arg) in evidence.argv.iter().enumerate() {
        put(&format!("argv.{index}.utf8.hex"), hex(arg.as_bytes()));
    }
    for (name, bytes) in [
        ("stdin", &evidence.stdin),
        ("stdout", &evidence.capture.stdout),
        ("stderr", &evidence.capture.stderr),
    ] {
        put(&format!("{name}.file"), format!("{name}.bin"));
        put(&format!("{name}.bytes"), bytes.len().to_string());
    }
    put(
        "elapsed.ns",
        evidence.capture.elapsed.as_nanos().to_string(),
    );
    match &evidence.capture.termination {
        Termination::Exited {
            code,
            success,
            status_debug,
        } => {
            put("termination.kind", "exited".into());
            put(
                "exit.code",
                code.map_or_else(|| "none".into(), |code| code.to_string()),
            );
            put("exit.success", success.to_string());
            put("exit.status_debug.utf8.hex", hex(status_debug.as_bytes()));
        }
        Termination::TimedOut {
            cleanup_code,
            cleanup_success,
            ready_seen,
            alive_at_timeout,
            reaped,
            cleanup_error,
            cleanup_status_debug,
        } => {
            put("termination.kind", "timed-out".into());
            put(
                "cleanup.code",
                cleanup_code.map_or_else(|| "none".into(), |code| code.to_string()),
            );
            put(
                "cleanup.success",
                cleanup_success.map_or_else(|| "none".into(), |success| success.to_string()),
            );
            put("timeout.ready_seen", ready_seen.to_string());
            put("timeout.alive_at_boundary", alive_at_timeout.to_string());
            put("cleanup.reaped", reaped.to_string());
            put(
                "cleanup.error.utf8.hex",
                hex(cleanup_error.as_deref().unwrap_or("").as_bytes()),
            );
            put(
                "cleanup.status_debug.utf8.hex",
                hex(cleanup_status_debug.as_deref().unwrap_or("").as_bytes()),
            );
        }
    }
    put("expected.exit", evidence.expected.exit_code.to_string());
    put("expected.stdout.hex", hex(&evidence.expected.stdout));
    put("expected.stderr.hex", hex(&evidence.expected.stderr));
    let verdict = evidence.verdict();
    put("verdict.stdout_matches", verdict.stdout_matches.to_string());
    put("verdict.stderr_matches", verdict.stderr_matches.to_string());
    put("verdict.exit_matches", verdict.exit_matches.to_string());
    put("verdict.passed", verdict.passed.to_string());
    Ok(fields
        .into_iter()
        .map(|(key, value)| format!("{key}\t{value}\n"))
        .collect::<String>()
        .into_bytes())
}

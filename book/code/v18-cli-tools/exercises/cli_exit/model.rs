//! Result-to-process-boundary model; provided bytes are not a Lua print interceptor.
use lua_core::Value;

pub const USAGE: &str = "usage: lua_app [-i] [-v] [-e stat] [-l name] [--] [script [args]]";

#[derive(Clone)]
pub enum CliError {
    Usage(String),
    Runtime(String),
}

#[derive(Clone)]
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub result: Result<Vec<Value>, CliError>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Transcript {
    pub code: u8,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub type Finish = fn(Outcome) -> Transcript;

#[allow(dead_code)] // Fixed faults use this helper; a completed learner may not.
pub fn error_transcript(outcome: Outcome, usage_code: u8, runtime_code: u8) -> Transcript {
    let (code, stderr) = match outcome.result {
        Ok(_) => (0, Vec::new()),
        Err(CliError::Usage(message)) => (
            usage_code,
            format!("lua: {message}\n{USAGE}\n").into_bytes(),
        ),
        Err(CliError::Runtime(message)) => (runtime_code, format!("lua: {message}\n").into_bytes()),
    };
    Transcript {
        code,
        stdout: outcome.stdout,
        stderr,
    }
}

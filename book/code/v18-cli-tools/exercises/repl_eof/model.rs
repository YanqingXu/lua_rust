//! The hook owns only pending source and a diagnostic; it cannot access a VM.
use lua_compiler::{compile, CompileError, CompileErrorKind};

pub struct PendingInput {
    pub source: String,
    pub diagnostic: Option<CompileError>,
}

impl PendingInput {
    pub fn idle() -> Self {
        Self {
            source: String::new(),
            diagnostic: None,
        }
    }

    pub fn incomplete(source: &str) -> Self {
        let error = compile(source.as_bytes()).unwrap_err();
        assert_eq!(error.kind, CompileErrorKind::NeedMoreInput);
        Self {
            source: source.to_owned(),
            diagnostic: Some(error),
        }
    }
}

pub type Finish = fn(&mut PendingInput) -> Result<(), CompileError>;

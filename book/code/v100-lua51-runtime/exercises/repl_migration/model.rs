//! A local projection of pending input; the real controller owns its own state.
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
        let diagnostic = compile(source.as_bytes()).unwrap_err();
        assert_eq!(diagnostic.kind, CompileErrorKind::NeedMoreInput);
        Self {
            source: source.to_owned(),
            diagnostic: Some(diagnostic),
        }
    }
}

pub type Finish = fn(&mut PendingInput) -> Result<(), CompileError>;

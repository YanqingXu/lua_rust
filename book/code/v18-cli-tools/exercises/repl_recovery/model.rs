//! 真实 Session/编译器；框架拥有一次创建的 Session，hook 只管理 source 生命周期。
use lua_app::{AppError, Session};
use lua_compiler::CompileError;
use lua_core::Value;

pub enum Event {
    Blank,
    Incomplete(CompileError),
    Values(Vec<Value>),
    Error(AppError),
}

impl Event {
    pub fn describe(&self) -> String {
        match self {
            Self::Blank => "blank".into(),
            Self::Incomplete(error) => format!("incomplete: {}", error.message),
            Self::Values(values) => format!("values: {values:?}"),
            Self::Error(AppError::Compile(error)) => format!("compile error: {}", error.message),
            Self::Error(error) => format!("error: {error}"),
        }
    }
}

pub type Submit = fn(&mut Session, &mut String, &str) -> Event;

pub struct Repl {
    session: Session,
    submit: Submit,
    pub source: String,
    pub submitted: usize,
}

impl Repl {
    pub fn new(submit: Submit) -> Self {
        Self {
            session: Session::new(),
            submit,
            source: String::new(),
            submitted: 0,
        }
    }

    pub fn push(&mut self, line: &str) -> Event {
        self.submitted += 1;
        (self.submit)(&mut self.session, &mut self.source, line)
    }

    /// 宿主直接访问同一 Session，仅供观察被污染 source 之外的副作用。
    pub fn inspect(&mut self, source: &[u8]) -> Result<Vec<Value>, AppError> {
        self.session.execute_multi(source)
    }
}

// ANCHOR: repl_recovery_append
pub fn append_line(source: &mut String, line: &str) -> bool {
    if source.is_empty() && line.trim().is_empty() {
        return false;
    }
    if source.is_empty() && line.trim_start().starts_with('=') {
        source.push_str("return ");
        source.push_str(line.trim_start().trim_start_matches('='));
    } else {
        source.push_str(line);
    }
    source.push('\n');
    true
}
// ANCHOR_END: repl_recovery_append

// ANCHOR: repl_recovery_classifier
pub fn is_incomplete(error: &CompileError) -> bool {
    error.message.contains("expected 'end'")
        || error.message.contains("unexpected end")
        || error.message.contains("unterminated")
}

pub fn attempt(session: &mut Session, source: &str) -> Event {
    match session.execute_multi(source.as_bytes()) {
        Ok(values) => Event::Values(values),
        Err(AppError::Compile(error)) if is_incomplete(&error) => Event::Incomplete(error),
        Err(error) => Event::Error(error),
    }
}
// ANCHOR_END: repl_recovery_classifier

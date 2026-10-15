use lua_bytecode::{render, ViewFormat};
use lua_compiler::{compile, CompileError, CompileErrorKind};
use lua_core::{ByteString, Value};
use lua_vm::{Vm, VmError};
use std::fmt;
use std::io::{BufRead, Write};

#[derive(Debug)]
pub enum AppError {
    Compile(CompileError),
    Runtime(VmError),
    Bytecode(String),
    Host(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "{error}"),
            Self::Runtime(error) => write!(formatter, "{error}"),
            Self::Bytecode(error) => write!(formatter, "{error}"),
            Self::Host(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AppError {}

pub fn execute(source: &[u8]) -> Result<Value, AppError> {
    Ok(Session::new()
        .execute_multi(source)?
        .into_iter()
        .next()
        .unwrap_or(Value::Nil))
}

pub fn execute_multi(source: &[u8]) -> Result<Vec<Value>, AppError> {
    Session::new().execute_multi(source)
}

pub fn compile_and_disassemble(source: &[u8]) -> Result<String, AppError> {
    let proto = compile(source).map_err(AppError::Compile)?;
    render(&proto, ViewFormat::Text).map_err(AppError::Bytecode)
}

pub struct Session {
    vm: Vm,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self { vm: Vm::new() }
    }

    pub fn execute_multi(&mut self, source: &[u8]) -> Result<Vec<Value>, AppError> {
        let proto = compile(source).map_err(AppError::Compile)?;
        self.vm.execute_multi(&proto).map_err(AppError::Runtime)
    }

    pub fn set_arg_table(&mut self, entries: &[(i64, Vec<u8>)]) -> Result<(), AppError> {
        self.vm
            .set_arg_table(entries)
            .map_err(|error| AppError::Host(error.to_string()))
    }

    pub fn require(&mut self, module: &[u8]) -> Result<(), AppError> {
        self.vm
            .set_global(b"__cli_module", Value::String(ByteString::new(module)))
            .map_err(|error| AppError::Host(error.to_string()))?;
        let result = self.execute_multi(b"require(__cli_module)");
        let cleanup = self
            .vm
            .set_global(b"__cli_module", Value::Nil)
            .map_err(|error| AppError::Host(error.to_string()));
        result?;
        cleanup
    }
}

/// One interactive input owner borrows the same VM session across completed chunks.
pub struct Repl<'a> {
    session: &'a mut Session,
    source: String,
    pending: Option<CompileError>,
}

#[derive(Debug)]
pub enum ReplEvent {
    Blank,
    NeedMoreInput,
    Values(Vec<Value>),
}

impl<'a> Repl<'a> {
    pub fn new(session: &'a mut Session) -> Self {
        Self {
            session,
            source: String::new(),
            pending: None,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    // ANCHOR: repl_structured_submit
    pub fn submit(&mut self, line: &str) -> Result<ReplEvent, AppError> {
        self.submit_fragment(line, true)
    }

    fn submit_fragment(&mut self, line: &str, terminated: bool) -> Result<ReplEvent, AppError> {
        if self.source.is_empty() && line.trim().is_empty() {
            return Ok(ReplEvent::Blank);
        }
        if self.source.is_empty() {
            if let Some(expression) = line.trim_start().strip_prefix('=') {
                self.source.push_str("return ");
                self.source.push_str(expression);
            } else {
                self.source.push_str(line);
            }
        } else {
            self.source.push_str(line);
        }
        if terminated {
            self.source.push('\n');
        }
        match self.session.execute_multi(self.source.as_bytes()) {
            Err(AppError::Compile(error)) if error.kind == CompileErrorKind::NeedMoreInput => {
                self.pending = Some(error);
                Ok(ReplEvent::NeedMoreInput)
            }
            result => {
                self.source.clear();
                self.pending = None;
                result.map(ReplEvent::Values)
            }
        }
    }
    // ANCHOR_END: repl_structured_submit

    // ANCHOR: repl_structured_eof
    pub fn finish_eof(&mut self) -> Result<(), AppError> {
        self.source.clear();
        match self.pending.take() {
            Some(error) => Err(AppError::Compile(error)),
            None => Ok(()),
        }
    }
    // ANCHOR_END: repl_structured_eof
}

/// The transport distinguishes EOF (Ok(0)) from an input error (Err).
pub fn run_repl(
    session: &mut Session,
    mut input: impl BufRead,
    mut output: impl Write,
    mut errors: impl Write,
    prompts: bool,
) -> Result<(), AppError> {
    if prompts {
        writeln!(output, "Lua 5.1 (lua-rust teaching v0.18)").map_err(output_error)?;
    }
    let mut repl = Repl::new(session);
    let mut line = String::new();
    loop {
        if prompts {
            write!(output, "{}", if repl.is_pending() { ">> " } else { "> " })
                .map_err(output_error)?;
            output.flush().map_err(output_error)?;
        }
        // ANCHOR: repl_structured_transport
        line.clear();
        let count = input
            .read_line(&mut line)
            .map_err(|error| AppError::Host(format!("cannot read REPL input: {error}")))?;
        if count == 0 {
            if prompts {
                writeln!(output).map_err(output_error)?;
            }
            return repl.finish_eof();
        }
        let terminated = line.ends_with('\n');
        if terminated {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        match repl.submit_fragment(&line, terminated) {
            Ok(ReplEvent::Blank | ReplEvent::NeedMoreInput) => {}
            Ok(ReplEvent::Values(values)) => write_values(&mut output, &values)?,
            Err(error) => writeln!(errors, "lua: {error}").map_err(output_error)?,
        }
        // ANCHOR_END: repl_structured_transport
    }
}

fn output_error(error: std::io::Error) -> AppError {
    AppError::Host(format!("cannot write REPL output: {error}"))
}

fn write_values(output: &mut impl Write, values: &[Value]) -> Result<(), AppError> {
    if values.is_empty() {
        return Ok(());
    }
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t").map_err(output_error)?;
        }
        match value {
            Value::String(bytes) => output.write_all(bytes.as_bytes()).map_err(output_error)?,
            other => output
                .write_all(other.to_string().as_bytes())
                .map_err(output_error)?,
        }
    }
    output.write_all(b"\n").map_err(output_error)
}

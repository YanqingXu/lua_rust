use lua_bytecode::{render, ViewFormat};
use lua_compiler::{compile, CompileError};
use lua_core::{ByteString, Value};
use lua_vm::{Vm, VmError};
use std::fmt;

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

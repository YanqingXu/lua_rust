use lua_compiler::{compile, CompileError};
use lua_core::{disassemble, Value};
use lua_vm::{Vm, VmError};
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Compile(CompileError),
    Runtime(VmError),
    Bytecode(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "{error}"),
            Self::Runtime(error) => write!(formatter, "{error}"),
            Self::Bytecode(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AppError {}

pub fn execute(source: &[u8]) -> Result<Value, AppError> {
    let proto = compile(source).map_err(AppError::Compile)?;
    Vm::new().execute(&proto).map_err(AppError::Runtime)
}

pub fn execute_multi(source: &[u8]) -> Result<Vec<Value>, AppError> {
    let proto = compile(source).map_err(AppError::Compile)?;
    Vm::new().execute_multi(&proto).map_err(AppError::Runtime)
}

pub fn compile_and_disassemble(source: &[u8]) -> Result<String, AppError> {
    let proto = compile(source).map_err(AppError::Compile)?;
    disassemble(&proto).map_err(AppError::Bytecode)
}

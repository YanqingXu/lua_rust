use crate::model::PendingInput;
use lua_compiler::CompileError;

pub fn discard_pending(input: &mut PendingInput) -> Result<(), CompileError> {
    input.source.clear();
    input.diagnostic = None;
    Ok(())
}

pub fn clone_without_consuming(input: &mut PendingInput) -> Result<(), CompileError> {
    match input.diagnostic.clone() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

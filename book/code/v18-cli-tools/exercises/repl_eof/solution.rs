use crate::model::PendingInput;
use lua_compiler::CompileError;

// ANCHOR: repl_eof_solution
pub fn finish(input: &mut PendingInput) -> Result<(), CompileError> {
    input.source.clear();
    match input.diagnostic.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
// ANCHOR_END: repl_eof_solution

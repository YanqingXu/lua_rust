use crate::model::PendingInput;
use lua_compiler::CompileError;

// ANCHOR: repl_eof_starter
pub fn finish(input: &mut PendingInput) -> Result<(), CompileError> {
    // TODO: EOF must consume the pending diagnostic exactly once, then clear source.
    let _ = input;
    Ok(())
}
// ANCHOR_END: repl_eof_starter

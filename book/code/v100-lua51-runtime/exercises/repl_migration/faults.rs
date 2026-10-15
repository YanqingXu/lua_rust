use crate::model::PendingInput;
use lua_compiler::CompileError;

// ANCHOR: repl_migration_faults
pub fn historical_message_heuristic(error: &CompileError) -> bool {
    error.message.contains("expected 'end'")
        || error.message.contains("unexpected end")
        || error.message.contains("unterminated")
}

pub fn silent_eof(input: &mut PendingInput) -> Result<(), CompileError> {
    input.source.clear();
    input.diagnostic = None;
    Ok(())
}

pub fn clone_eof(input: &mut PendingInput) -> Result<(), CompileError> {
    match input.diagnostic.clone() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
// ANCHOR_END: repl_migration_faults

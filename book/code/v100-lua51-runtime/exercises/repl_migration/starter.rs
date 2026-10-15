use crate::model::PendingInput;
use lua_compiler::CompileError;

// ANCHOR: repl_migration_starter
pub fn finish(input: &mut PendingInput) -> Result<(), CompileError> {
    // TODO: transfer the original diagnostic once and release the pending source.
    let _ = input;
    Ok(())
}
// ANCHOR_END: repl_migration_starter

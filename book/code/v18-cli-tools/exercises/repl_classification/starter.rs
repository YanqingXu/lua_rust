use crate::model::Decision;
use lua_compiler::CompileError;
use lua_core::Proto;

// ANCHOR: repl_classification_starter
pub fn decide(compilation: &Result<Proto, CompileError>) -> Decision {
    // TODO: inspect the structured error kind, preserving only NeedMoreInput.
    match compilation {
        Ok(_) => Decision::Execute,
        Err(_) => Decision::Reject,
    }
}
// ANCHOR_END: repl_classification_starter

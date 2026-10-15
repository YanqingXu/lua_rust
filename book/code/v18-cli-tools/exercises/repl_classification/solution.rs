use crate::model::Decision;
use lua_compiler::{CompileError, CompileErrorKind};
use lua_core::Proto;

// ANCHOR: repl_classification_solution
pub fn decide(compilation: &Result<Proto, CompileError>) -> Decision {
    match compilation {
        Ok(_) => Decision::Execute,
        Err(error) if error.kind == CompileErrorKind::NeedMoreInput => Decision::Preserve,
        Err(_) => Decision::Reject,
    }
}
// ANCHOR_END: repl_classification_solution

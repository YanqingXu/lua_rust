use crate::model::Decision;
use lua_compiler::CompileError;
use lua_core::Proto;

// ANCHOR: repl_classification_faults
pub fn keywords(compilation: &Result<Proto, CompileError>) -> Decision {
    match compilation {
        Ok(_) => Decision::Execute,
        Err(error)
            if error.message.contains("expected 'end'")
                || error.message.contains("unexpected end")
                || error.message.contains("unterminated") =>
        {
            Decision::Preserve
        }
        Err(_) => Decision::Reject,
    }
}

pub fn eof_position(compilation: &Result<Proto, CompileError>, source_len: usize) -> Decision {
    match compilation {
        Ok(_) => Decision::Execute,
        Err(error) if error.position.offset == source_len => Decision::Preserve,
        Err(_) => Decision::Reject,
    }
}
// ANCHOR_END: repl_classification_faults

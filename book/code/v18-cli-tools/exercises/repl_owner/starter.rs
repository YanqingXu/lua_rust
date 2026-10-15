use lua_app::{AppError, Session};
use lua_core::Value;

// ANCHOR: repl_owner_starter
pub fn submit(_session: &mut Session, source: &[u8]) -> Result<Vec<Value>, AppError> {
    // TODO: keep the caller's owner alive across submissions.
    lua_app::execute_multi(source)
}
// ANCHOR_END: repl_owner_starter

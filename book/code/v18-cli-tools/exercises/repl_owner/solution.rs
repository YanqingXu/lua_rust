use lua_app::{AppError, Session};
use lua_core::Value;

// ANCHOR: repl_owner_solution
pub fn submit(session: &mut Session, source: &[u8]) -> Result<Vec<Value>, AppError> {
    session.execute_multi(source)
}
// ANCHOR_END: repl_owner_solution

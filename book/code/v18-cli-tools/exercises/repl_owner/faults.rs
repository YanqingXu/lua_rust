use lua_app::{AppError, Session};
use lua_core::Value;

pub fn fresh_session(_session: &mut Session, source: &[u8]) -> Result<Vec<Value>, AppError> {
    lua_app::execute_multi(source)
}

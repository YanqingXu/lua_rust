use crate::model::{append_line, attempt, Event};
use lua_app::Session;

// ANCHOR: repl_recovery_starter
pub fn submit(session: &mut Session, source: &mut String, line: &str) -> Event {
    if !append_line(source, line) {
        return Event::Blank;
    }
    let event = attempt(session, source);
    // TODO：Incomplete 要保留；成功/普通编译错误/运行错误都清空并继续。
    source.clear();
    event
}
// ANCHOR_END: repl_recovery_starter

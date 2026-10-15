use crate::model::{append_line, attempt, Event};
use lua_app::Session;

// ANCHOR: repl_recovery_solution
pub fn submit(session: &mut Session, source: &mut String, line: &str) -> Event {
    if !append_line(source, line) {
        return Event::Blank;
    }
    match attempt(session, source) {
        event @ Event::Incomplete(_) => event,
        event => {
            source.clear();
            event
        }
    }
}
// ANCHOR_END: repl_recovery_solution

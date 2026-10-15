use crate::model::{append_line, attempt, Event, Repl};
use lua_app::{AppError, Session};

pub fn clear_every_line(session: &mut Session, source: &mut String, line: &str) -> Event {
    if !append_line(source, line) {
        return Event::Blank;
    }
    let event = attempt(session, source);
    source.clear();
    event
}

pub fn keep_after_error(session: &mut Session, source: &mut String, line: &str) -> Event {
    if !append_line(source, line) {
        return Event::Blank;
    }
    let event = attempt(session, source);
    if matches!(event, Event::Values(_)) {
        source.clear();
    }
    event
}

fn ordinary_error_as_result(event: Event) -> Result<(), AppError> {
    match event {
        Event::Error(error) => Err(error),
        _ => Ok(()),
    }
}

// ANCHOR: repl_recovery_early_return
pub fn stop_at_first_error(repl: &mut Repl, lines: &[&str]) -> Result<(), AppError> {
    for line in lines {
        ordinary_error_as_result(repl.push(line))?;
    }
    Ok(())
}
// ANCHOR_END: repl_recovery_early_return

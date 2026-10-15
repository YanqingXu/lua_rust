mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "implement the EOF ownership TODO in starter.rs"]
fn eof_reports_pending_once_and_releases_buffer() {
    contracts::eof_reports_pending_once_and_releases_buffer(starter::finish);
}

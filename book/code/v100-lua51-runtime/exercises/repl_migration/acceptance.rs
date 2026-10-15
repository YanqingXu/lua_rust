mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "implement the local snapshot EOF migration TODO in starter.rs"]
fn migrated_eof_consumes_original_diagnostic_once() {
    contracts::migrated_eof_consumes_original_diagnostic_once(starter::finish);
}

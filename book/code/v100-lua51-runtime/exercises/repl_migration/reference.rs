mod contracts;
mod faults;
mod model;
mod probes;
mod solution;

#[test]
fn migrated_eof_consumes_original_diagnostic_once() {
    contracts::migrated_eof_consumes_original_diagnostic_once(solution::finish);
}

#[test]
fn historical_message_heuristic_misses_structured_kind() {
    probes::historical_message_heuristic_misses_structured_kind();
}

#[test]
fn silent_eof_drops_original_diagnostic() {
    probes::silent_eof_drops_original_diagnostic();
}

#[test]
fn cloned_eof_diagnostic_is_reported_twice() {
    probes::cloned_eof_diagnostic_is_reported_twice();
}

#[test]
fn local_v100_controller_executes_once_and_recovers() {
    probes::local_v100_controller_executes_once_and_recovers();
}

#[test]
fn local_v100_driver_distinguishes_final_fragment_from_newline() {
    probes::local_v100_driver_distinguishes_final_fragment_from_newline();
}

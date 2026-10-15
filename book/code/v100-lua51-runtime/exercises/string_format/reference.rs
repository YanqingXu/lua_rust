mod contracts;
mod faults;
mod model;
mod probes;
mod solution;

#[test]
fn argument_cursor_commits_only_after_validation() {
    contracts::argument_cursor_commits_only_after_validation(solution::consume);
}

#[test]
fn to_string_does_not_implement_default_float_precision() {
    probes::to_string_does_not_implement_default_float_precision();
}

#[test]
fn one_byte_specifier_loses_flags_width_and_precision() {
    probes::one_byte_specifier_loses_flags_width_and_precision();
}

#[test]
fn stringify_accepts_a_table_that_percent_s_must_reject() {
    probes::stringify_accepts_a_table_that_percent_s_must_reject();
}

#[test]
fn advancing_before_validation_commits_a_failed_argument() {
    probes::advancing_before_validation_commits_a_failed_argument();
}

#[test]
fn actual_percent_escape_does_not_consume_an_argument() {
    probes::actual_percent_escape_does_not_consume_an_argument();
}

#[test]
fn actual_parser_checks_the_whole_bounded_specification() {
    probes::actual_parser_checks_the_whole_bounded_specification();
}

#[test]
fn actual_vm_applies_padding_precision_and_existing_conversions() {
    probes::actual_vm_applies_padding_precision_and_existing_conversions();
}

#[test]
fn actual_failure_returns_no_partial_string_and_session_remains_usable() {
    probes::actual_failure_returns_no_partial_string_and_session_remains_usable();
}

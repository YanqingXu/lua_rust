mod contracts;
mod faults;
mod model;
mod probes;
mod solution;

#[test]
fn retry_positions_keep_greedy_order_and_the_repetition_minimum() {
    contracts::retry_positions_keep_greedy_order_and_the_repetition_minimum(
        solution::next_position,
    );
}

#[test]
fn plain_and_large_init_need_distinct_boundaries() {
    probes::plain_and_large_init_need_distinct_boundaries();
}

#[test]
fn literal_contains_cannot_match_byte_atoms_and_escaped_classes() {
    probes::literal_contains_cannot_match_byte_atoms_and_escaped_classes();
}

#[test]
fn actual_atom_scanning_and_fixed_suffix_show_why_greedy_retries() {
    probes::actual_atom_scanning_and_fixed_suffix_show_why_greedy_retries();
}

#[test]
fn plus_needs_one_byte_and_an_empty_match_is_still_some_range() {
    probes::plus_needs_one_byte_and_an_empty_match_is_still_some_range();
}

#[test]
fn an_unreached_malformed_suffix_is_not_an_eager_parse_error() {
    probes::an_unreached_malformed_suffix_is_not_an_eager_parse_error();
}

#[test]
fn actual_atom_parser_borrows_sets_and_matches_raw_bytes() {
    probes::actual_atom_parser_borrows_sets_and_matches_raw_bytes();
}

#[test]
fn a_limit_error_is_not_a_successful_no_match_result() {
    probes::a_limit_error_is_not_a_successful_no_match_result();
}

#[test]
fn actual_vm_maps_half_open_ranges_and_recovers_after_a_find_error() {
    probes::actual_vm_maps_half_open_ranges_and_recovers_after_a_find_error();
}

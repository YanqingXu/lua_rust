mod contracts;
mod faults;
mod model;
mod probes;
mod solution;

#[test]
fn checkpoint_restore_replaces_old_slots_and_removes_new_slots() {
    contracts::checkpoint_restore_replaces_old_slots_and_removes_new_slots(solution::restore_slots);
}

#[test]
fn truncate_or_clear_cannot_restore_mutated_existing_slots() {
    probes::truncate_or_clear_cannot_restore_mutated_existing_slots();
}

#[test]
fn restoring_a_work_counter_refunds_work_that_was_already_spent() {
    probes::restoring_a_work_counter_refunds_work_that_was_already_spent();
}

#[test]
fn actual_backtracking_restores_capture_ends_before_retrying_the_suffix() {
    probes::actual_backtracking_restores_capture_ends_before_retrying_the_suffix();
}

#[test]
fn actual_nested_empty_and_position_captures_keep_distinct_kinds() {
    probes::actual_nested_empty_and_position_captures_keep_distinct_kinds();
}

#[test]
fn actual_candidate_and_next_find_start_with_independent_capture_slots() {
    probes::actual_candidate_and_next_find_start_with_independent_capture_slots();
}

#[test]
fn actual_vm_returns_owned_binary_capture_values() {
    probes::actual_vm_returns_owned_binary_capture_values();
}

#[test]
fn actual_capture_and_work_limits_return_errors_without_destroying_the_session() {
    probes::actual_capture_and_work_limits_return_errors_without_destroying_the_session();
}

#[test]
fn actual_checkpoint_restore_matches_the_teaching_projection() {
    probes::actual_checkpoint_restore_matches_the_teaching_projection();
}

#[path = "../exercises/string_find_captures/contracts.rs"]
mod contracts;
#[path = "../exercises/string_find_captures/faults.rs"]
mod faults;
#[path = "../exercises/string_find_captures/model.rs"]
mod model;
#[path = "../exercises/string_find_captures/probes.rs"]
mod probes;
#[path = "../exercises/string_find_captures/solution.rs"]
mod solution;

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if !matches!(mode.as_str(), "faults" | "captures" | "all") {
        return Err("mode must be faults|captures|all".into());
    }
    if matches!(mode.as_str(), "faults" | "all") {
        probes::truncate_or_clear_cannot_restore_mutated_existing_slots();
        println!("[ok] design truncate fault removes new slots but keeps mutated old contents");
        println!("[ok] design clear fault loses captures opened before the retry");
        probes::restoring_a_work_counter_refunds_work_that_was_already_spent();
        println!("[ok] design budget snapshot fault refunds work already spent");
    }
    if matches!(mode.as_str(), "captures" | "all") {
        contracts::checkpoint_restore_replaces_old_slots_and_removes_new_slots(
            solution::restore_slots,
        );
        println!("[ok] reference restore replaces Open Closed and Position slot contents");
        println!(
            "[ok] reference restore removes appended slots and preserves the borrowed checkpoint"
        );
        println!("[ok] reference checkpoint can restore more than one failed attempt");
        probes::actual_backtracking_restores_capture_ends_before_retrying_the_suffix();
        println!("[ok] actual greedy retry closes the capture at the successful shorter boundary");
        probes::actual_nested_empty_and_position_captures_keep_distinct_kinds();
        println!(
            "[ok] actual nested opening order and empty versus position kinds remain distinct"
        );
        probes::actual_candidate_and_next_find_start_with_independent_capture_slots();
        println!("[ok] actual next candidate and next call start with independent capture slots");
        probes::actual_vm_returns_owned_binary_capture_values();
        println!("[ok] actual binary capture Values survive later GC and Session drop");
        probes::actual_capture_and_work_limits_return_errors_without_destroying_the_session();
        println!("[ok] actual capture and work limits stay errors and the Session remains usable");
        probes::actual_checkpoint_restore_matches_the_teaching_projection();
        println!("[ok] actual restore API agrees with complete slot replacement");
    }
    Ok(())
}

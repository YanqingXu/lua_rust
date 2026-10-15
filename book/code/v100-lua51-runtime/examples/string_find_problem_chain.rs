#[path = "../exercises/string_find/contracts.rs"]
mod contracts;
#[path = "../exercises/string_find/faults.rs"]
mod faults;
#[path = "../exercises/string_find/model.rs"]
mod model;
#[path = "../exercises/string_find/probes.rs"]
mod probes;
#[path = "../exercises/string_find/solution.rs"]
mod solution;

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if !matches!(mode.as_str(), "faults" | "find" | "all") {
        return Err("mode must be faults|find|all".into());
    }
    if matches!(mode.as_str(), "faults" | "all") {
        probes::plain_and_large_init_need_distinct_boundaries();
        println!("[ok] fixed ignored plain treats a literal dot as an atom");
        println!("[ok] fixed large-init rejection loses a successful terminal empty range");
        probes::literal_contains_cannot_match_byte_atoms_and_escaped_classes();
        println!("[ok] fixed literal contains misses byte atoms classes and escapes");
        probes::actual_atom_scanning_and_fixed_suffix_show_why_greedy_retries();
        println!("[ok] fixed greedy-only retry misses aaab matched by a-star-ab");
        probes::an_unreached_malformed_suffix_is_not_an_eager_parse_error();
        println!("[ok] fixed eager suffix parsing errors before the subject reaches it");
    }
    if matches!(mode.as_str(), "find" | "all") {
        contracts::retry_positions_keep_greedy_order_and_the_repetition_minimum(
            solution::next_position,
        );
        println!("[ok] reference retry positions descend from greedy end without repetition");
        println!("[ok] reference star includes zero and plus preserves its one-byte minimum");
        probes::plus_needs_one_byte_and_an_empty_match_is_still_some_range();
        println!("[ok] actual empty match is Some range and maps to Lua start one end zero");
        probes::actual_atom_parser_borrows_sets_and_matches_raw_bytes();
        println!("[ok] actual parser borrows bracket sets and matches byte classes");
        println!("[ok] actual byte matching preserves NUL and non-UTF8 subject bytes");
        probes::a_limit_error_is_not_a_successful_no_match_result();
        println!("[ok] actual search resource error stays distinct from no match");
        probes::actual_vm_maps_half_open_ranges_and_recovers_after_a_find_error();
        println!("[ok] actual VM maps half-open ranges into one-based inclusive results");
        println!("[ok] actual find error leaves the same Session usable");
    }
    Ok(())
}

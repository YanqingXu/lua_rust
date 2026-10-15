#[path = "../exercises/string_format/contracts.rs"]
mod contracts;
#[path = "../exercises/string_format/faults.rs"]
mod faults;
#[path = "../exercises/string_format/model.rs"]
mod model;
#[path = "../exercises/string_format/probes.rs"]
mod probes;
#[path = "../exercises/string_format/solution.rs"]
mod solution;

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if !matches!(mode.as_str(), "faults" | "format" | "all") {
        return Err("mode must be faults|format|all".into());
    }
    if matches!(mode.as_str(), "faults" | "all") {
        probes::to_string_does_not_implement_default_float_precision();
        println!("[ok] fixed to_string gives 3.5 while actual percent-f gives 3.500000");
        probes::one_byte_specifier_loses_flags_width_and_precision();
        println!("[ok] fixed one-byte specifier reads zero instead of parsing 08.2f");
        probes::stringify_accepts_a_table_that_percent_s_must_reject();
        println!("[ok] fixed stringify accepts a table that actual percent-s rejects");
        probes::advancing_before_validation_commits_a_failed_argument();
        println!("[ok] fixed eager cursor advance skips rejected and missing arguments");
    }
    if matches!(mode.as_str(), "format" | "all") {
        contracts::argument_cursor_commits_only_after_validation(solution::consume);
        println!("[ok] reference accepts numeric strings and string-compatible numbers");
        println!("[ok] reference consumes exactly one value after successful validation");
        println!("[ok] reference preserves cursor and borrowed storage on argument errors");
        probes::actual_percent_escape_does_not_consume_an_argument();
        println!("[ok] actual literal percent consumes no argument and missing input errors");
        probes::actual_parser_checks_the_whole_bounded_specification();
        println!("[ok] actual parser handles flags width precision and following cursor");
        println!("[ok] actual parser rejects overlong incomplete and unknown specifications");
        probes::actual_vm_applies_padding_precision_and_existing_conversions();
        println!("[ok] actual VM applies numeric padding string precision and integer conversion");
        probes::actual_failure_returns_no_partial_string_and_session_remains_usable();
        println!(
            "[ok] actual VM returns an error without a partial result and keeps Session usable"
        );
    }
    Ok(())
}

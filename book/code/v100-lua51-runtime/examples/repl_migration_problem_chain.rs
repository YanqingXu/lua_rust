#[path = "../exercises/repl_migration/contracts.rs"]
mod contracts;
#[path = "../exercises/repl_migration/faults.rs"]
mod faults;
#[path = "../exercises/repl_migration/model.rs"]
mod model;
#[path = "../exercises/repl_migration/probes.rs"]
mod probes;
#[path = "../exercises/repl_migration/solution.rs"]
mod solution;

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if !matches!(mode.as_str(), "faults" | "migration" | "all") {
        return Err("mode must be faults|migration|all".into());
    }
    if matches!(mode.as_str(), "faults" | "all") {
        probes::historical_message_heuristic_misses_structured_kind();
        println!("[ok] historical message heuristic misses a completable expression");
        probes::silent_eof_drops_original_diagnostic();
        println!("[ok] fixed silent EOF loses the original diagnostic");
        probes::cloned_eof_diagnostic_is_reported_twice();
        println!("[ok] fixed clone EOF repeats the diagnostic and retains source");
    }
    if matches!(mode.as_str(), "migration" | "all") {
        contracts::migrated_eof_consumes_original_diagnostic_once(solution::finish);
        println!("[ok] local reference idle EOF succeeds");
        println!("[ok] local reference preserves diagnostic kind position and message");
        println!("[ok] local reference consumes pending EOF exactly once");
        probes::local_v100_controller_executes_once_and_recovers();
        println!("[ok] v1.0 pending input executes no prefix");
        println!("[ok] v1.0 completed input executes exactly once");
        println!("[ok] v1.0 invalid input clears buffer and keeps the same Session");
        println!("[ok] v1.0 pending EOF consumes state without executing its prefix");
        probes::local_v100_driver_distinguishes_final_fragment_from_newline();
        println!("[ok] v1.0 no-LF short string returns the original pending EOF error");
        println!("[ok] v1.0 short string LF rejects and final complete fragment runs");
    }
    Ok(())
}

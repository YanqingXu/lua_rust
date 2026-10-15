#[path = "../exercises/repl_eof/contracts.rs"]
mod contracts;
#[path = "../exercises/repl_eof/faults.rs"]
mod faults;
#[path = "../exercises/repl_eof/model.rs"]
mod model;
#[path = "../exercises/support/process.rs"]
mod process;
#[path = "../exercises/repl_eof/real.rs"]
mod real;
#[path = "../exercises/repl_eof/solution.rs"]
mod solution;

fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if !matches!(mode.as_str(), "faults" | "eof" | "real" | "all") {
        return Err("mode must be faults|eof|real|all".into());
    }
    if matches!(mode.as_str(), "faults" | "all") {
        let mut silent = model::PendingInput::incomplete("return 40 +\n");
        assert!(faults::discard_pending(&mut silent).is_ok());
        println!("[ok] fixed silent-drop reports success for pending EOF");
        let mut repeated = model::PendingInput::incomplete("return 40 +\n");
        let first = faults::clone_without_consuming(&mut repeated).unwrap_err();
        let second = faults::clone_without_consuming(&mut repeated).unwrap_err();
        assert_eq!(first, second);
        assert!(!repeated.source.is_empty());
        println!("[ok] fixed clone repeats diagnostic and retains pending source");
    }
    if matches!(mode.as_str(), "eof" | "all") {
        contracts::eof_reports_pending_once_and_releases_buffer(solution::finish);
        println!("[ok] reference idle EOF succeeds");
        println!("[ok] reference pending EOF returns original kind position and message");
        println!("[ok] reference EOF consumes diagnostic and clears source exactly once");
    }
    if matches!(mode.as_str(), "real" | "all") {
        for case in real::eof_cases(&process::example_binary()?)? {
            println!(
                "[ok] {}: exit={} stdout={} stderr={}",
                case.name,
                case.exit,
                case.stdout.len(),
                case.stderr.len()
            );
        }
    }
    Ok(())
}

#[path = "../exercises/support/bytecode_process.rs"]
pub mod bytecode_process;
#[path = "../exercises/bytecode_trace/faults.rs"]
mod faults;
#[path = "../exercises/bytecode_trace/model.rs"]
pub mod model;
#[path = "../exercises/support/process.rs"]
pub mod process;
#[path = "../exercises/bytecode_trace/real.rs"]
pub mod real;
#[path = "../exercises/bytecode_trace/solution.rs"]
mod solution;

fn timing() -> Result<(), String> {
    for (label, hook) in [
        (
            "before: record after executing",
            faults::record_after as model::Dispatch,
        ),
        (
            "after: clone values before executing",
            solution::dispatch as model::Dispatch,
        ),
    ] {
        println!("{label}");
        for (step, event) in model::run(hook)?.iter().enumerate() {
            println!(
                "  step={step} pc={} line={} depth={} op={:?} registers={:?}",
                event.pc, event.line, event.frame_depth, event.opcode, event.registers
            );
        }
    }
    println!(
        "before: aliased slot history={:?}",
        faults::alias_history()?
    );
    Ok(())
}

fn vm() -> Result<(), String> {
    let (result, events) = real::arithmetic_events()?;
    println!(
        "real Vm: result={result:?} pcs={:?} depths={:?}",
        events.iter().map(|event| event.pc).collect::<Vec<_>>(),
        events
            .iter()
            .map(|event| event.frame_depth)
            .collect::<Vec<_>>()
    );
    let (result, events) = real::branch_events()?;
    println!(
        "real branch: result={result:?} pcs={:?} event_count={}",
        events.iter().map(|event| event.pc).collect::<Vec<_>>(),
        events.len()
    );
    println!(
        "real controls: default/accumulate/take-off/enable-clear/moved={:?}",
        real::control_counts()?
    );
    let failed = real::failure_boundaries()?;
    println!(
        "real failure events: LoadK={} invalid={} budget2={} budget0={} failed_budget_pc={}",
        failed.load_events.len(),
        failed.invalid_events.len(),
        failed.budget_events.len(),
        failed.zero_budget_events,
        failed.budget.pc
    );
    println!(
        "real identity: {:?}",
        real::identity_is_not_deep_snapshot_or_gc_root()?
    );
    Ok(())
}

fn cli() -> Result<(), String> {
    let binary = bytecode_process::example_binary()?;
    println!("real binary: {:?}", real::binary_cases(&binary)?);
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "timing" => timing(),
        "vm" => vm(),
        "cli" => cli(),
        "trace-json" => {
            let (_, events) = real::arithmetic_events()?;
            print!("{}", lua_bytecode::trace_json(&events));
            Ok(())
        }
        "all" => {
            timing()?;
            vm()?;
            cli()
        }
        _ => Err("usage: bytecode_trace_problem_chain [timing|vm|cli|trace-json|all]".into()),
    }
}

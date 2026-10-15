//! Fixed faulty/correct designs; learner starter changes do not alter this replay.
#[path = "../exercises/continuation/faults.rs"]
mod faults;
#[path = "../exercises/continuation/model.rs"]
mod model;
#[path = "../exercises/continuation/real.rs"]
mod real;
#[path = "../exercises/continuation/solution.rs"]
mod solution;

use model::{ContinuationVm, Design, ResumePc, SaveFrame, TurnVm};

fn healthy() -> Design {
    Design {
        save: solution::save_frame,
        pc: solution::resume_pc,
    }
}

fn storage() {
    for (label, save) in [
        ("before shared-scratch", faults::shared_scratch as SaveFrame),
        ("after per-coroutine", solution::save_frame as SaveFrame),
    ] {
        let mut vm = ContinuationVm::new(Design { save, ..healthy() }, &[(1, 10), (2, 100)]);
        for (id, input) in [(1, 0), (2, 0), (1, 1), (2, 2), (1, 3), (2, 4)] {
            let result = vm.resume(id, input);
            println!("{label} co={id} input={input}: {result:?}");
        }
        println!("  entry-effects={:?}", vm.starts);
    }
}

fn pc() {
    for (label, pc) in [
        ("before restart", faults::restart_entry as ResumePc),
        ("after next-pc", solution::resume_pc as ResumePc),
    ] {
        let mut vm = ContinuationVm::new(Design { pc, ..healthy() }, &[(1, 10)]);
        for input in [0, 5, 7] {
            let result = vm.resume(1, input);
            println!("{label} input={input}: {result:?}");
        }
        println!("{label} entry-effects={}", vm.starts[&1]);
    }
}

fn turn() {
    for (label, run) in [
        (
            "before shared-turn",
            faults::share_turn_channels as fn(&mut TurnVm) -> model::ChildYield,
        ),
        (
            "after owner-turn",
            solution::run_child as fn(&mut TurnVm) -> model::ChildYield,
        ),
    ] {
        let mut vm = TurnVm::new();
        let child = run(&mut vm);
        println!(
            "{label}: observed-parent={:?} child-empty={} child={child:?}",
            vm.observed_parent, vm.child_saw_empty_channels
        );
        println!(
            "  restored owner={} statuses={:?} values={:?} frames={:?}",
            vm.current, vm.statuses, vm.yield_values, vm.yield_frames
        );
    }
}

fn real() {
    println!("real interleaved: {:?}", real::interleaved_locals());
    println!(
        "real nested-yields: {:?}",
        real::nested_calls_and_multiple_yields()
    );
    println!(
        "real nested-turn: {:?}",
        real::nested_statuses_and_value_direction()
    );
}

fn main() {
    let selected = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match selected.as_str() {
        "storage" => storage(),
        "pc" => pc(),
        "turn" => turn(),
        "real" => real(),
        "all" => {
            storage();
            pc();
            turn();
            real();
        }
        _ => {
            eprintln!("usage: continuation_problem_chain [storage|pc|turn|real|all]");
            std::process::exit(2);
        }
    }
}

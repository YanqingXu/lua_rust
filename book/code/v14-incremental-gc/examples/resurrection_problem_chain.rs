//! 固定重放：不读取 starter，成功退出不表示完成 TODO。
#[path = "../exercises/resurrection/faults.rs"]
mod faults;
#[path = "../exercises/resurrection/model.rs"]
mod model;
#[path = "../exercises/resurrection/real.rs"]
mod real;
#[path = "../exercises/resurrection/solution.rs"]
mod solution;

use model::{Cycle, ResurrectionHeap};

fn report(label: &str, cycle: &Cycle) {
    println!("{label}: pending={} weak={:?} callback_saw={:?} live={:?} calls={} rescue={} finalized={:?} prefix={}",
        cycle.pending_before_callback, cycle.weak_before_callback, cycle.callback_saw_weak,
        cycle.live_after_sweep, cycle.total_callbacks, cycle.rescue_root,
        cycle.finalized, cycle.prefix_live);
}

fn order() {
    for (label, atomic) in [
        (
            "before cleanup-first",
            faults::cleanup_too_early as model::AtomicOrder,
        ),
        (
            "after pending-first",
            solution::finish_atomic as model::AtomicOrder,
        ),
    ] {
        let mut heap = ResurrectionHeap::default();
        report(label, &heap.cycle(atomic, true));
        for line in &heap.trace {
            println!("  {line}");
        }
    }
}

fn cycles() {
    let mut heap = ResurrectionHeap::default();
    report("rescue cycle=1", &heap.cycle(solution::finish_atomic, true));
    report("rescue cycle=2", &heap.cycle(solution::finish_atomic, true));
    heap.remove_rescue_root();
    report("rescue cycle=3", &heap.cycle(solution::finish_atomic, true));
    let mut heap = ResurrectionHeap::default();
    report(
        "no-rescue cycle=1",
        &heap.cycle(solution::finish_atomic, false),
    );
    report(
        "no-rescue cycle=2",
        &heap.cycle(solution::finish_atomic, false),
    );
}

fn real() {
    for rescue in [true, false] {
        for (index, cycle) in real::heap_cycles(rescue).iter().enumerate() {
            println!("real Heap rescue={rescue} cycle={}: weak={:?} nil={:?} accessible={:?} finalized={:?} events={} prefix={}",
                index + 1, cycle.weak, cycle.weak_nil, cycle.accessible, cycle.finalized,
                cycle.total_events, cycle.prefix_live);
        }
    }
    println!(
        "real VM setup/rescue/release={:?}",
        real::vm_rescue_cycles()
    );
}

fn main() {
    let selected = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match selected.as_str() {
        "order" => order(),
        "cycles" => cycles(),
        "real" => real(),
        "all" => {
            order();
            cycles();
            real();
        }
        _ => {
            eprintln!("usage: resurrection_problem_chain [order|cycles|real|all]");
            std::process::exit(2);
        }
    }
}

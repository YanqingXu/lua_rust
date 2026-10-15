//! 固定循环反例与参考协议，不读取 learner starter。
#[path = "../exercises/cycles/faults.rs"]
mod faults;
#[path = "../exercises/cycles/model.rs"]
mod model;
#[path = "../exercises/cycles/real.rs"]
mod real;
#[path = "../exercises/cycles/solution.rs"]
mod solution;

use model::CycleVm;

fn cycles() {
    for entry in ["self", "A"] {
        for (label, run) in [
            ("before late", faults::publish_too_late as model::RunLoader),
            ("after early", solution::run_loader as model::RunLoader),
        ] {
            let mut vm = CycleVm::new(run, 6);
            let result = vm.require(entry);
            println!(
                "{label} entry={entry}: result={result:?} calls={:?}",
                vm.calls
            );
            if let Ok(exports) = result {
                println!(
                    "  name={:?} answer={:?} inner={:?}",
                    vm.field(&exports, "name"),
                    vm.field(&exports, "answer"),
                    vm.field(&exports, "inner")
                );
                let cached = vm.require(entry).unwrap();
                println!("  final cache identity={}", cached == exports);
                if entry == "A" {
                    let b = vm.require("B").unwrap();
                    println!(
                        "  A.inner==B={} B.name={:?} B.answer={:?} B.inner={:?}",
                        vm.field(&exports, "inner") == Ok(b.clone()),
                        vm.field(&b, "name"),
                        vm.field(&b, "answer"),
                        vm.field(&b, "inner")
                    );
                }
            }
            for line in &vm.trace {
                println!("  {line}");
            }
        }
    }
}

fn truthy() {
    let mut vm = CycleVm::new(solution::run_loader, 6);
    let partial = vm.require("partial").unwrap();
    println!(
        "partial: ready={:?} saw_unready={:?} same={:?}",
        vm.field(&partial, "ready"),
        vm.field(&partial, "saw_unready"),
        vm.field(&partial, "same")
    );
    let first = vm.require("done_true");
    let second = vm.require("done_true");
    println!(
        "completed true: first={first:?} second={second:?} calls={:?}",
        vm.calls
    );
}

fn real() {
    println!("real preload self+pair={:?}", real::vm_preload_cycles());
    println!(
        "real preload partial+true={:?}",
        real::vm_explicit_partial_and_completed_true()
    );
    println!("real files exports/after-gc={:?}", real::vm_file_cycles());
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "cycles" => cycles(),
        "truthy" => truthy(),
        "real" => real(),
        "all" => {
            cycles();
            truthy();
            real();
        }
        _ => {
            eprintln!("usage: require_cycle_problem_chain [cycles|truthy|real|all]");
            std::process::exit(2);
        }
    }
}

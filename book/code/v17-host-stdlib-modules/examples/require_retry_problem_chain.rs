//! 固定重放 require 失败恢复协议，不读取 learner starter。
#[path = "../exercises/retry/faults.rs"]
mod faults;
#[path = "../exercises/retry/model.rs"]
pub mod model;
#[path = "../exercises/retry/real.rs"]
mod real;
#[path = "../exercises/retry/solution.rs"]
mod solution;

fn retry() {
    for (label, attempt) in [
        (
            "before question-mark",
            faults::question_mark as model::Attempt,
        ),
        ("after current-key", solution::attempt as model::Attempt),
    ] {
        let mut vm = model::RetryVm::new(attempt);
        let first = vm.require("broken", model::fail);
        let failed = first.is_err();
        println!(
            "{label}: first_failed={failed} cached_after_failure={}",
            vm.cached("broken")
        );
        let second = vm.require("broken", model::repaired).unwrap();
        println!(
            "{label}: retry={second} runs={} effects={}",
            vm.calls["broken"], vm.side_effects
        );
    }
}

fn dependency() {
    for (label, attempt) in [
        ("before clear-all", faults::clear_all as model::Attempt),
        ("after current-key", solution::attempt as model::Attempt),
    ] {
        let mut vm = model::RetryVm::new(attempt);
        vm.require("parent", model::depends_then_fails).unwrap_err();
        println!(
            "{label}: parent={} dependency={} outer={}",
            vm.cached("parent"),
            vm.cached("dependency"),
            vm.cached("outer")
        );
        let value = vm.require("parent", model::depends_then_succeeds).unwrap();
        println!(
            "{label}: retry={value} dependency_runs={}",
            vm.calls["dependency"]
        );
    }
}

fn real() {
    println!("real preload same-execute={}", real::preload_retry());
    println!(
        "real dependency preserved={}",
        real::nested_dependency_survives_parent_failure()
    );
    println!(
        "real require-yield boundary restored={}",
        real::rejected_yield_then_retry_and_yield()
    );
    println!(
        "real missing-file before/retry={:?}",
        real::missing_file_can_be_created_and_retried()
    );
    println!(
        "real compile-stop before/retry={:?}",
        real::compile_error_stops_before_a_later_template_then_retries()
    );
    println!(
        "real runtime-file before/retry={:?}",
        real::runtime_failure_clears_marker_then_same_vm_retries()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "retry" => retry(),
        "dependency" => dependency(),
        "real" => real(),
        "all" => {
            retry();
            dependency();
            real();
        }
        _ => {
            eprintln!("usage: require_retry_problem_chain [retry|dependency|real|all]");
            std::process::exit(2);
        }
    }
}

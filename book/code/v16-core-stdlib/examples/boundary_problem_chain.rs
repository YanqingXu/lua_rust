//! Fixed faults and reference hooks; learner starter changes do not affect replay.
#[path = "../exercises/boundary/faults.rs"]
mod faults;
#[path = "../exercises/boundary/model.rs"]
mod model;
#[path = "../exercises/boundary/real.rs"]
mod real;
#[path = "../exercises/boundary/solution.rs"]
mod solution;

use model::{BoundaryHook, BoundaryVm};

fn errors() {
    for (label, hook) in [
        (
            "before question-mark",
            faults::early_question_mark as BoundaryHook,
        ),
        (
            "after capture-result",
            solution::invoke_boundary as BoundaryHook,
        ),
    ] {
        let mut vm = BoundaryVm::new(hook, 0);
        let result = vm.invoke(model::ordinary_error);
        println!("{label}: {}", vm.summary(&result));
        let later = vm.try_yield(7);
        println!("  later legal-yield: {}", vm.summary(&later));
    }
    let mut prefixed = BoundaryVm::new(solution::invoke_boundary, 3);
    let result = prefixed.invoke(model::success);
    println!("after nonzero-prefix: {}", prefixed.summary(&result));
}

fn nested() {
    for (label, hook) in [
        ("before boolean", faults::depth_as_boolean as BoundaryHook),
        ("after depth", solution::invoke_boundary as BoundaryHook),
    ] {
        let mut vm = BoundaryVm::new(hook, 0);
        let result = vm.invoke(model::nested_after_caught_error);
        println!("{label}: {}", vm.summary(&result));
        for line in &vm.trace {
            println!("  {line}");
        }
    }
    let mut vm = BoundaryVm::new(solution::invoke_boundary, 0);
    let denied = vm.invoke(model::yielding_callback);
    println!("after rejected callback: {}", vm.summary(&denied));
}

fn real() {
    println!(
        "real pcall then legal-yield: {:?}",
        real::pcall_then_yield()
    );
    println!(
        "real xpcall then legal-yield: {:?}",
        real::xpcall_then_yield()
    );
    println!("real sort then legal-yield: {:?}", real::sort_then_yield());
    println!(
        "real __call then legal-yield: {:?}",
        real::callable_metamethod_then_yield()
    );
    println!(
        "real nested pcall then legal-yield: {:?}",
        real::nested_pcall_then_yield()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "error" => errors(),
        "nested" => nested(),
        "real" => real(),
        "all" => {
            errors();
            nested();
            real();
        }
        _ => {
            eprintln!("usage: boundary_problem_chain [error|nested|real|all]");
            std::process::exit(2);
        }
    }
}

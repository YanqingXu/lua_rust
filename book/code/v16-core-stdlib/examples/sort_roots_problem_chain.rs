//! 固定 proposal/反例重放，不读取 learner starter，也不把真实 VM 已知失败算作成功。
#[path = "../exercises/sort_roots/faults.rs"]
mod faults;
#[path = "../exercises/sort_roots/model.rs"]
mod model;
#[path = "../exercises/sort_roots/real.rs"]
mod real;
#[path = "../exercises/sort_roots/solution.rs"]
mod solution;

use lua_core::Value;
use model::{key, Scenario, SortVm};

fn liveness() {
    for (label, scope) in [
        (
            "before current-pair",
            faults::current_pair_only as model::Scope,
        ),
        (
            "proposal full-snapshot",
            solution::with_snapshot as model::Scope,
        ),
    ] {
        let mut vm = SortVm::new(Scenario::ClearAndCollect);
        let result = vm.run_sort(scope);
        let accessible = vm
            .items
            .iter()
            .map(|item| vm.heap.table_get(*item, &key("rank")).is_ok())
            .collect::<Vec<_>>();
        println!(
            "{label}: result={result:?} comparisons={} accessible={accessible:?} roots={:?}",
            vm.comparisons, vm.temporary_roots
        );
        if result.is_ok() {
            println!("  sorted ranks={:?}", vm.source_ranks());
        }
        vm.clear_source().unwrap();
        vm.collect().unwrap();
        println!(
            "  after release accessible={:?}",
            vm.items
                .iter()
                .map(|item| vm.heap.table_get(*item, &key("rank")).is_ok())
                .collect::<Vec<_>>()
        );
    }
}

fn errors() {
    for (label, scope) in [
        (
            "before question-mark",
            faults::early_question_mark as model::Scope,
        ),
        (
            "proposal saved-result",
            solution::with_snapshot as model::Scope,
        ),
    ] {
        let mut vm = SortVm::new(Scenario::ErrorAfterMutation);
        let result = vm.run_sort(scope);
        vm.collect().unwrap();
        println!(
            "{label}: result={result:?} root_len={} source[1]={:?} replaced_item_access={:?}",
            vm.temporary_roots.len(),
            vm.heap.table_get(vm.source, &Value::Number(1.0)),
            vm.heap.table_get(vm.items[0], &key("rank"))
        );
        println!(
            "  prefix={:?}",
            vm.heap.table_get(vm.prefix, &key("sentinel"))
        );
    }
}

fn real() {
    println!(
        "real VM unchanged-source/truthiness={}",
        real::vm_normal_sort_and_truthiness()
    );
    println!(
        "real VM error-mutation={}",
        real::vm_comparator_error_keeps_its_source_mutation()
    );
    println!("known source-clearing failure is separate: examples/sort_snapshot_roots_gap.lua");
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "liveness" => liveness(),
        "errors" => errors(),
        "real" => real(),
        "all" => {
            liveness();
            errors();
            real();
        }
        _ => {
            eprintln!("usage: sort_roots_problem_chain [liveness|errors|real|all]");
            std::process::exit(2);
        }
    }
}

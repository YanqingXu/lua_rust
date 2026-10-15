//! 固定前后对照；不读取 learner starter。
#[path = "../exercises/globals/faults.rs"]
mod faults;
#[path = "../exercises/globals/model.rs"]
mod model;
#[path = "../exercises/globals/real.rs"]
mod real;
#[path = "../exercises/globals/solution.rs"]
mod solution;

use lua_core::Value;
use model::GlobalVm;

fn coherence() {
    for (label, install) in [
        (
            "before snapshot",
            faults::install_snapshot as model::Install,
        ),
        ("after shared", solution::install as model::Install),
    ] {
        let mut vm = GlobalVm::new(install);
        vm.set_global("forward", Value::Number(12.0)).unwrap();
        vm.set_through_g("reverse", Value::Number(34.0)).unwrap();
        println!(
            "{label}: bare.forward={:?} _G.forward={:?} bare.reverse={:?} _G.reverse={:?}",
            vm.global("forward"),
            vm.through_g("forward"),
            vm.global("reverse"),
            vm.through_g("reverse")
        );
        vm.set_global("seed", Value::Nil).unwrap();
        println!(
            "{label}: delete bare.seed={:?} _G.seed={:?}",
            vm.global("seed"),
            vm.through_g("seed")
        );
    }
}

fn identity() {
    let mut vm = GlobalVm::new(solution::install);
    let owner = vm.environment.owner_table();
    println!(
        "initial _G._G identity={}",
        vm.through_g("_G") == Ok(Value::Table(owner))
    );
    let replacement = vm.self_cycle();
    vm.set_global("_G", Value::Table(replacement)).unwrap();
    vm.set_global("bare", Value::Number(77.0)).unwrap();
    println!(
        "replace _G: owner_unchanged={} bare={:?} _G.bare={:?}",
        vm.environment.owner_table() == owner,
        vm.global("bare"),
        vm.through_g("bare")
    );
    vm.set_global("_G", Value::Nil).unwrap();
    vm.collect();
    println!(
        "nil _G: bare={:?} owner_access={:?}",
        vm.global("bare"),
        vm.heap.table_get(owner, &model::key("bare"))
    );
}

fn gc() {
    let mut vm = GlobalVm::new(solution::install);
    let payload = vm.self_cycle();
    let garbage = vm.self_cycle();
    vm.set_global("payload", Value::Table(payload)).unwrap();
    vm.collect();
    println!(
        "rooted env: payload_access={} copied_garbage_live={:?}",
        vm.heap.table_get(payload, &model::key("answer")) == Ok(Value::Number(42.0)),
        vm.heap.is_live(garbage)
    );
    vm.set_global("payload", Value::Nil).unwrap();
    vm.collect();
    println!(
        "delete payload: exact_nil={} payload_live={:?}",
        vm.global("payload") == Ok(Value::Nil),
        vm.heap.is_live(payload)
    );
    println!(
        "standalone _G self-cycle kept/env-dead/child-dead={:?}",
        real::heap_self_referential_environment_needs_a_root()
    );
}

fn real() {
    println!("real VM bidirectional={}", real::vm_bidirectional());
    println!(
        "real VM install/keep/release={:?}",
        real::vm_reachable_graph_across_execute_and_gc()
    );
    println!(
        "real VM replace/nil/still-usable={:?}",
        real::vm_rebinding_g_preserves_environment()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "coherence" => coherence(),
        "identity" => identity(),
        "gc" => gc(),
        "real" => real(),
        "all" => {
            coherence();
            identity();
            gc();
            real();
        }
        _ => {
            eprintln!("usage: globals_problem_chain [coherence|identity|gc|real|all]");
            std::process::exit(2);
        }
    }
}

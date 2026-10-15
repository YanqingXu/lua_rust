//! 固定重放，不读取 learner starter。
#[path = "../exercises/roots/faults.rs"]
mod faults;
#[path = "../exercises/roots/model.rs"]
mod model;
#[path = "../exercises/roots/real.rs"]
mod real;
#[path = "../exercises/roots/solution.rs"]
mod solution;

fn mirror() {
    for (label, mirror) in [
        ("before entry-only", faults::entry_only as model::Mirror),
        ("after full-mirror", solution::mirror_roots as model::Mirror),
    ] {
        let mut vm = model::VmModel::suspended();
        vm.collect(mirror, solution::extra_direct_roots, true);
        println!("{label}: {} resume={:?}", vm.state(), vm.resume_check());
    }
    let mut fresh = model::VmModel::not_started();
    fresh.collect(solution::mirror_roots, solution::extra_direct_roots, true);
    println!(
        "fresh entry: {} entry_graph_live={}",
        fresh.state(),
        fresh.is_live(3) && fresh.is_live(14)
    );
}

fn release() {
    for (label, direct) in [
        (
            "before scheduler+all-upvalues",
            faults::root_scheduler_and_all_upvalues as model::Direct,
        ),
        (
            "before all-upvalues-only",
            faults::root_all_upvalues as model::Direct,
        ),
        (
            "after active-only",
            solution::extra_direct_roots as model::Direct,
        ),
    ] {
        let mut vm = model::VmModel::suspended();
        vm.collect(solution::mirror_roots, direct, false);
        let graph = model::SUSPENDED_GRAPH.map(|id| vm.is_live(id));
        println!("{label}: {} graph_live={graph:?}", vm.state());
    }
}

fn real() {
    for mirror in [false, true] {
        for (index, state) in real::heap_mirror_then_release(mirror).iter().enumerate() {
            println!(
                "real Heap mirror={mirror} checkpoint={index}: thread={} graph={:?} prefix={}",
                state.thread, state.graph, state.prefix
            );
        }
    }
    println!(
        "real VM resume={}",
        real::vm_keeps_entry_and_suspended_values()
    );
    println!(
        "real VM release states/decreased/weak-nil={:?}",
        real::vm_unreachable_cycle_is_pruned()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "mirror" => mirror(),
        "release" => release(),
        "real" => real(),
        "all" => {
            mirror();
            release();
            real();
        }
        _ => {
            eprintln!("usage: coroutine_roots_problem_chain [mirror|release|real|all]");
            std::process::exit(2);
        }
    }
}

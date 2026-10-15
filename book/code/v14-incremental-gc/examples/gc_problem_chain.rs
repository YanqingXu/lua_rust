//! cargo run --example gc_problem_chain -- split|progress|barrier|real
#[path = "../exercises/model.rs"]
mod model;

use lua_core::{GcStep, Heap, Value};
use model::{chain, pair};

fn report(label: &str, step: GcStep) {
    let done = step.completed.map_or_else(
        || "none".to_owned(),
        |stats| format!("{}/{}/{}", stats.before, stats.collected, stats.remaining),
    );
    println!(
        "{label}: phase={:?} work={} completed={done}",
        step.phase, step.work
    );
}

fn split() {
    println!("[split] A->B->C, D->D; roots=[A]; budget=1");
    let (mut before, a, _, _, _) = chain();
    report("before collect-as-step", before.collect(&[a]));
    let (mut after, a, _, _, _) = chain();
    report("after one step", after.step(&[a], 1));
    println!("after {}", after.state());
}

fn progress() {
    println!("[progress] A->B->C, D->D; roots=[A]; budget=1");
    let (mut before, a, _, _, _) = chain();
    for call in 1..=3 {
        report(
            &format!("before call={call}"),
            before.step_restarting(&[a], 1),
        );
        println!("before {}", before.state());
    }
    let (mut after, a, _, _, _) = chain();
    for call in 1..=8 {
        report(&format!("after call={call}"), after.step(&[a], 1));
        println!("after {}", after.state());
    }
}

fn barrier() {
    println!("[barrier] A and B allocated before cycle; roots=[A]");
    for enabled in [false, true] {
        let label = if enabled {
            "after barrier"
        } else {
            "before raw-write"
        };
        let (mut heap, a, b) = pair();
        heap.step(&[a], 1);
        println!("{label} scanned: {}", heap.state());
        if enabled {
            heap.set_edge(a, b);
        } else {
            heap.set_edge_without_barrier(a, b);
        }
        println!("{label} A->B: {}", heap.state());
        report(label, heap.collect(&[a]));
        println!("{label} child_live={}", heap.is_live(b));
    }
}

fn real() {
    println!("[real] final v0.14 Heap; A->B->C, D->D; roots=[A]; budget=1");
    let mut heap = Heap::new();
    let a = heap.allocate_table().unwrap();
    let b = heap.allocate_table().unwrap();
    let c = heap.allocate_table().unwrap();
    let d = heap.allocate_table().unwrap();
    heap.table_set(a, Value::Number(1.0), Value::Table(b))
        .unwrap();
    heap.table_set(b, Value::Number(1.0), Value::Table(c))
        .unwrap();
    heap.table_set(d, Value::Number(1.0), Value::Table(d))
        .unwrap();
    for call in 1..=8 {
        report(
            &format!("real call={call}"),
            heap.step(&[Value::Table(a)], &[], 1).unwrap(),
        );
    }
    let mut heap = Heap::new();
    let a = heap.allocate_table().unwrap();
    let b = heap.allocate_table().unwrap();
    heap.step(&[Value::Table(a)], &[], 1).unwrap();
    heap.table_set(a, Value::Number(1.0), Value::Table(b))
        .unwrap();
    let stats = heap.collect(&[Value::Table(a)], &[]).unwrap();
    println!(
        "real barrier: collected={} remaining={} child_live={}",
        stats.collected,
        stats.remaining,
        heap.is_live(b).unwrap()
    );
}

fn main() {
    let selected = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match selected.as_str() {
        "split" => split(),
        "progress" => progress(),
        "barrier" => barrier(),
        "real" => real(),
        "all" => {
            split();
            progress();
            barrier();
            real();
        }
        _ => {
            eprintln!("usage: gc_problem_chain [split|progress|barrier|real|all]");
            std::process::exit(2);
        }
    }
}

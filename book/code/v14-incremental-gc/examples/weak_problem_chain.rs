//! Fixed faults/solutions; running this example does not complete learner TODOs.
#[path = "../exercises/weak/faults.rs"]
mod faults;
#[path = "../exercises/weak/model.rs"]
mod model;
#[path = "../exercises/weak/real.rs"]
mod real;
#[path = "../exercises/weak/solution.rs"]
mod solution;

use model::{Classifier, Mode, Propagator};

fn show_weak(label: &str, mode: Mode, root_key: bool, classify: Classifier) {
    let mut heap = model::classification(mode);
    let roots = if root_key { vec![1, 2] } else { vec![1] };
    heap.collect(&roots, classify, solution::propagate);
    println!(
        "{label} {mode:?} root_key={root_key}: live={:?} array={} map={}",
        heap.labels(),
        heap.tables[&1].array.len(),
        heap.tables[&1].map.len()
    );
}

fn weak() {
    show_weak(
        "before all-strong",
        Mode::Values,
        false,
        faults::all_edges_strong,
    );
    for mode in [Mode::Strong, Mode::Values, Mode::Keys, Mode::Both] {
        show_weak("after classify", mode, false, solution::classify_edges);
    }
    show_weak("after classify", Mode::Keys, true, solution::classify_edges);
}

fn show_chain(label: &str, propagate: Propagator) {
    let mut heap = model::chain();
    heap.collect(&[1, 2, 3], solution::classify_edges, propagate);
    println!(
        "{label}: live={:?} V2_accessible={}",
        heap.labels(),
        heap.tables.contains_key(&6)
    );
    for line in &heap.trace {
        println!("  {line}");
    }
}

fn ephemeron() {
    show_chain("before one-round", faults::one_round);
    show_chain("after fixed-point", solution::propagate);
    for (label, classify) in [
        (
            "before all-strong cycle",
            faults::all_edges_strong as Classifier,
        ),
        ("after gated cycle", solution::classify_edges as Classifier),
    ] {
        let mut heap = model::unrooted_cycle();
        heap.collect(&[1], classify, solution::propagate);
        println!(
            "{label}: live={:?} map={}",
            heap.labels(),
            heap.tables[&1].map.len()
        );
    }
}

fn real() {
    for mode in [b"v".as_slice(), b"k".as_slice(), b"kv".as_slice()] {
        println!(
            "real mode={}: {:?}",
            String::from_utf8_lossy(mode),
            real::mode_report(mode, false)
        );
    }
    println!("real k rooted-key: {:?}", real::mode_report(b"k", true));
    println!("real chain live/released: {:?}", real::cross_table_chain());
    println!(
        "real cycle reclaimed/cleaned: {:?}",
        real::unrooted_key_value_cycle()
    );
    println!(
        "real shared-mode transition: {:?}",
        real::shared_mode_between_cycles()
    );
}

fn main() {
    let selected = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match selected.as_str() {
        "weak" => weak(),
        "ephemeron" => ephemeron(),
        "real" => real(),
        "all" => {
            weak();
            ephemeron();
            real();
        }
        _ => {
            eprintln!("usage: weak_problem_chain [weak|ephemeron|real|all]");
            std::process::exit(2);
        }
    }
}

//! Fixed faults and reference hooks: editing starter never changes this replay.
#[path = "../exercises/cache/faults.rs"]
mod faults;
#[path = "../exercises/cache/model.rs"]
mod model;
#[path = "../exercises/cache/real.rs"]
mod real;
#[path = "../exercises/cache/solution.rs"]
mod solution;

use lua_core::Value;
use model::{Design, Finish, Loader, Lookup, ModuleVm};

fn replay(label: &str, design: Design, loader: Loader) {
    let mut vm = ModuleVm::new(design);
    let first = vm.require(b"demo", &loader);
    let second = vm.require(b"demo", &loader);
    println!(
        "{label}: first={first}, second={second}, slot={}, calls={}, entry={:?}",
        vm.loaded.get(b"demo"),
        vm.calls,
        vm.entry_slots
    );
}

fn lookup() {
    for (label, hook) in [
        ("before never-cache", faults::never_cache as Lookup),
        ("after truthy-cache", solution::lookup as Lookup),
    ] {
        replay(
            label,
            Design {
                lookup: hook,
                finish: solution::finish,
            },
            Loader {
                publish: None,
                results: vec![Value::Number(42.0)],
            },
        );
    }
    for (label, hook) in [
        ("before existence-false", faults::existence_only as Lookup),
        ("after truthy-false", solution::lookup as Lookup),
    ] {
        replay(
            label,
            Design {
                lookup: hook,
                finish: solution::finish,
            },
            Loader {
                publish: None,
                results: vec![Value::Boolean(false)],
            },
        );
    }
}

fn finish() {
    for (label, hook) in [
        (
            "before ignore-publication",
            faults::ignore_publication as Finish,
        ),
        ("after preserve-publication", solution::finish as Finish),
    ] {
        replay(
            label,
            Design {
                lookup: solution::lookup,
                finish: hook,
            },
            Loader {
                publish: Some(Value::Number(17.0)),
                results: vec![Value::Nil],
            },
        );
    }
    for (label, hook) in [
        ("before scan-non-nil", faults::scan_non_nil as Finish),
        ("after first-only", solution::finish as Finish),
    ] {
        replay(
            label,
            Design {
                lookup: solution::lookup,
                finish: hook,
            },
            Loader {
                publish: None,
                results: vec![Value::Nil, Value::Number(99.0)],
            },
        );
    }
    replay(
        "after clear-slot",
        Design {
            lookup: solution::lookup,
            finish: solution::finish,
        },
        Loader {
            publish: Some(Value::Nil),
            results: vec![Value::Nil, Value::Number(99.0)],
        },
    );
}

fn real() {
    for probe in real::primitive_matrix() {
        assert_eq!(probe.actual, probe.expected, "{}", probe.name);
        println!("real {}: {:?}", probe.name, probe.actual);
    }
    let identity = real::table_identity_and_self_publication();
    assert_eq!(identity, vec![Value::Boolean(true); 2]);
    println!("real table identity + publication: {identity:?}");
    let owner = real::cached_table_across_execute_and_gc();
    assert_eq!(owner, vec![Value::Boolean(true); 4]);
    println!("real hidden loaded owner + execute/GC: {owner:?}");
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "lookup" => lookup(),
        "finish" => finish(),
        "real" => real(),
        "all" => {
            lookup();
            finish();
            real();
        }
        _ => {
            eprintln!("usage: require_cache_problem_chain [lookup|finish|real|all]");
            std::process::exit(2);
        }
    }
}

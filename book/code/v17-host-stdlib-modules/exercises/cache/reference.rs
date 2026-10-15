mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use lua_core::Value;
use model::{Design, Loader, ModuleVm};

#[test]
fn cache_hit_requires_a_truthy_value() {
    contracts::truthy_cache_lookup(solution::lookup);
}

#[test]
fn require_success_commits_only_the_first_result() {
    contracts::first_result_commit(solution::finish);
}

#[test]
fn missing_lookup_reexecutes_a_truthy_module() {
    let mut vm = ModuleVm::new(Design {
        lookup: faults::never_cache,
        finish: solution::finish,
    });
    let loader = Loader {
        publish: None,
        results: vec![Value::Number(42.0)],
    };
    assert_eq!(vm.require(b"demo", &loader), Value::Number(42.0));
    assert_eq!(vm.require(b"demo", &loader), Value::Number(42.0));
    assert_eq!(vm.calls, 2);
}

#[test]
fn existence_lookup_incorrectly_caches_false() {
    let mut vm = ModuleVm::new(Design {
        lookup: faults::existence_only,
        finish: solution::finish,
    });
    let loader = Loader {
        publish: None,
        results: vec![Value::Boolean(false)],
    };
    vm.require(b"demo", &loader);
    vm.require(b"demo", &loader);
    assert_eq!(vm.loaded.get(b"demo"), Value::Boolean(false));
    assert_eq!(vm.calls, 1);
}

#[test]
fn unconditional_fallback_overwrites_loader_publication() {
    let mut vm = ModuleVm::new(Design {
        lookup: solution::lookup,
        finish: faults::ignore_publication,
    });
    let loader = Loader {
        publish: Some(Value::Number(17.0)),
        results: vec![Value::Nil],
    };
    assert_eq!(vm.require(b"demo", &loader), Value::Boolean(true));
    assert_eq!(vm.loaded.get(b"demo"), Value::Boolean(true));
}

#[test]
fn scanning_results_uses_a_later_value_that_require_should_ignore() {
    let mut vm = ModuleVm::new(Design {
        lookup: solution::lookup,
        finish: faults::scan_non_nil,
    });
    let loader = Loader {
        publish: None,
        results: vec![Value::Nil, Value::Number(99.0)],
    };
    assert_eq!(vm.require(b"demo", &loader), Value::Number(99.0));
    assert_eq!(vm.loaded.get(b"demo"), Value::Number(99.0));
}

#[test]
fn real_vm_cache_primitive_and_first_result_matrix() {
    for probe in real::primitive_matrix() {
        assert_eq!(probe.actual, probe.expected, "{}", probe.name);
    }
}

#[test]
fn real_vm_table_identity_and_loader_publication_execute_once() {
    assert_eq!(
        real::table_identity_and_self_publication(),
        vec![Value::Boolean(true); 2]
    );
}

#[test]
fn real_vm_initial_loaded_owner_keeps_table_across_execute_and_gc() {
    assert_eq!(
        real::cached_table_across_execute_and_gc(),
        vec![Value::Boolean(true); 4]
    );
}

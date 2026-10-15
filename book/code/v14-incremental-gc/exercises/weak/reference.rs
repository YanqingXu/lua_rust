mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use model::Mode;
use real::WeakReport;

#[test]
fn weak_edges_follow_table_mode() {
    contracts::classifies_weak_edges(solution::classify_edges);
}

#[test]
fn ephemerons_reach_a_fixed_point_without_bootstrapping_cycles() {
    contracts::reaches_ephemeron_fixed_point(solution::propagate);
}

#[test]
fn faulty_strong_tracing_keeps_weak_children_and_bootstraps_the_cycle() {
    let mut weak = model::classification(Mode::Values);
    weak.collect(&[1], faults::all_edges_strong, solution::propagate);
    assert_eq!(weak.labels(), ["W", "K", "V", "A", "M"]);
    assert_eq!(weak.tables[&1].array.len(), 1);
    assert_eq!(weak.tables[&1].map.len(), 2);
    let mut cycle = model::unrooted_cycle();
    cycle.collect(&[1], faults::all_edges_strong, solution::propagate);
    assert_eq!(cycle.labels(), ["W", "K", "V"]);
}

#[test]
fn faulty_single_round_misses_the_second_table_value() {
    let mut chain = model::chain();
    chain.collect(&[1, 2, 3], solution::classify_edges, faults::one_round);
    assert_eq!(chain.labels(), ["W1", "W2", "K1", "V1", "K2"]);
    assert_eq!(chain.tables[&2].map.len(), 1);
    assert!(
        !chain.tables.contains_key(&6),
        "one round leaves a stale value in W2"
    );
}

#[test]
fn real_heap_weak_values_keep_keys_and_metatable_but_clean_values() {
    assert_eq!(
        real::mode_report(b"", false),
        WeakReport {
            key_accessible: true,
            value_accessible: true,
            array_value_accessible: true,
            metatable_accessible: true,
            map_entry: true,
            array_entry: true
        }
    );
    assert_eq!(
        real::mode_report(b"v", false),
        WeakReport {
            key_accessible: true,
            value_accessible: false,
            array_value_accessible: false,
            metatable_accessible: true,
            map_entry: false,
            array_entry: false
        }
    );
}

#[test]
fn real_heap_weak_keys_gate_map_values_and_keep_array_values() {
    assert_eq!(
        real::mode_report(b"k", false),
        WeakReport {
            key_accessible: false,
            value_accessible: false,
            array_value_accessible: true,
            metatable_accessible: true,
            map_entry: false,
            array_entry: true
        }
    );
    assert_eq!(
        real::mode_report(b"k", true),
        WeakReport {
            key_accessible: true,
            value_accessible: true,
            array_value_accessible: true,
            metatable_accessible: true,
            map_entry: true,
            array_entry: true
        }
    );
}

#[test]
fn real_heap_fully_weak_table_cleans_values_even_with_a_rooted_key() {
    assert_eq!(
        real::mode_report(b"kv", false),
        WeakReport {
            key_accessible: false,
            value_accessible: false,
            array_value_accessible: false,
            metatable_accessible: true,
            map_entry: false,
            array_entry: false
        }
    );
    assert_eq!(
        real::mode_report(b"kv", true),
        WeakReport {
            key_accessible: true,
            value_accessible: false,
            array_value_accessible: false,
            metatable_accessible: true,
            map_entry: false,
            array_entry: false
        }
    );
}

#[test]
fn real_heap_cross_table_ephemerons_reach_a_fixed_point_then_release() {
    assert_eq!(real::cross_table_chain(), (true, true));
}

#[test]
fn real_heap_unrooted_key_value_cycle_does_not_bootstrap() {
    assert_eq!(real::unrooted_key_value_cycle(), (true, true));
}

#[test]
fn real_heap_mode_change_refreshes_both_dependents_between_cycles() {
    assert_eq!(
        real::shared_mode_between_cycles(),
        real::TransitionReport {
            weak_with_external_roots: true,
            strong_keeps_both: true,
            weak_again_reclaims_both: true
        }
    );
}

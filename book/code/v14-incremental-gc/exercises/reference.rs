mod contracts;
mod model;
#[path = "reference/solution.rs"]
mod solution;

use lua_core::{GcPhase, Heap, TableRef, Value};
use model::{chain, pair};

#[test]
fn first_step_yields_before_collection_finishes() {
    contracts::bounded_first_step(solution::split_step);
}

#[test]
fn saved_progress_finishes_the_same_cycle() {
    contracts::progress_survives_calls(solution::resume_step);
}

#[test]
fn barrier_preserves_a_late_white_child() {
    contracts::late_edge_keeps_white_child(solution::connect);
}

#[test]
fn faulty_collect_wrapper_finishes_in_its_first_call() {
    let (mut heap, a, _, _, _) = chain();
    let result = heap.collect(&[a]);
    assert!(result.completed.is_some());
    assert!(result.work > 1);
}

#[test]
fn faulty_restart_repeats_the_first_mark() {
    let (mut heap, a, b, c, d) = chain();
    let mut states = Vec::new();
    for _ in 0..8 {
        let result = heap.step_restarting(&[a], 1);
        assert_eq!(
            (result.phase, result.work, result.completed),
            (GcPhase::Mark, 1, None)
        );
        assert!(heap.is_marked(a) && !heap.is_marked(b) && !heap.is_marked(c));
        assert!(heap.is_live(d));
        states.push(heap.state());
    }
    assert!(states.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn faulty_raw_write_reclaims_the_reachable_child() {
    let (mut heap, a, b) = pair();
    heap.step(&[a], 1);
    heap.set_edge_without_barrier(a, b);
    let stats = heap.collect(&[a]).completed.unwrap();
    assert_eq!((stats.collected, stats.remaining), (1, 1));
    assert!(heap.is_live(a) && !heap.is_live(b));
}

#[test]
fn real_heap_budget_one_matches_full_collection() {
    fn graph() -> (Heap, [Value; 1], [TableRef; 4]) {
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
        (heap, [Value::Table(a)], [a, b, c, d])
    }
    let (mut full, roots, _) = graph();
    let expected = full.collect(&roots, &[]).unwrap();
    let (mut heap, roots, [a, b, c, d]) = graph();
    let phases = [
        GcPhase::Mark,
        GcPhase::Mark,
        GcPhase::Atomic,
        GcPhase::Sweep,
        GcPhase::Sweep,
        GcPhase::Sweep,
        GcPhase::Sweep,
        GcPhase::Pause,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let result = heap.step(&roots, &[], 1).unwrap();
        assert_eq!((result.phase, result.work), (phase, 1));
        assert_eq!(result.completed, (index == 7).then_some(expected));
    }
    assert_eq!(
        (expected.before, expected.collected, expected.remaining),
        (4, 1, 3)
    );
    for table in [a, b, c] {
        assert_eq!(heap.is_live(table), Ok(true));
        assert!(heap.table_array_len(table).is_ok());
    }
    assert_eq!(heap.table_get(b, &Value::Number(1.0)), Ok(Value::Table(c)));
    assert_eq!(heap.table_array_len(c), Ok(0));
    assert_eq!(heap.is_live(d), Ok(false));
    assert_eq!(heap.collect(&[], &[]).unwrap().remaining, 0);
}

#[test]
fn real_heap_table_set_protects_a_preexisting_white_child() {
    let mut heap = Heap::new();
    let a = heap.allocate_table().unwrap();
    let b = heap.allocate_table().unwrap();
    let roots = [Value::Table(a)];
    assert_eq!(heap.step(&roots, &[], 1).unwrap().phase, GcPhase::Atomic);
    heap.table_set(a, Value::Number(1.0), Value::Table(b))
        .unwrap();
    assert_eq!(heap.collect(&roots, &[]).unwrap().collected, 0);
    assert_eq!(heap.is_live(b), Ok(true));
    assert_eq!(heap.table_array_len(b), Ok(0));
    assert_eq!(heap.table_get(a, &Value::Number(1.0)), Ok(Value::Table(b)));
    assert_eq!(heap.collect(&[], &[]).unwrap().remaining, 0);
}

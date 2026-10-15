//! Fixed faulty designs, independent of learner starter changes.
use crate::model::{Classifier, MarkHeap, Table, Value};

pub fn all_edges_strong(table: &Table) -> Vec<Value> {
    let mut edges = table.array.clone();
    for &(key, value) in &table.map {
        edges.extend([key, value]);
    }
    if let Some(metatable) = table.metatable {
        edges.push(Value::Ref(metatable));
    }
    edges
}

pub fn one_round(heap: &mut MarkHeap, classify: Classifier) {
    let candidates = heap.candidates();
    heap.record_round(&candidates);
    for value in candidates {
        heap.enqueue(value);
    }
    heap.drain(classify);
}

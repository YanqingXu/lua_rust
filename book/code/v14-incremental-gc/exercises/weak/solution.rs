use crate::model::{Classifier, MarkHeap, Table, Value};

// ANCHOR: weak_edges_solution
pub fn classify_edges(table: &Table) -> Vec<Value> {
    let mut edges = Vec::new();
    if !table.mode.weak_values() {
        edges.extend(table.array.iter().copied());
    }
    for &(key, value) in &table.map {
        if !table.mode.weak_keys() {
            edges.push(key);
        }
        if !table.mode.weak_keys() && !table.mode.weak_values() {
            edges.push(value);
        }
    }
    if let Some(metatable) = table.metatable {
        edges.push(Value::Ref(metatable));
    }
    edges
}
// ANCHOR_END: weak_edges_solution

// ANCHOR: ephemeron_solution
pub fn propagate(heap: &mut MarkHeap, classify: Classifier) {
    loop {
        let candidates = heap.candidates();
        heap.record_round(&candidates);
        if candidates.is_empty() {
            break;
        }
        for value in candidates {
            heap.enqueue(value);
        }
        heap.drain(classify);
    }
}
// ANCHOR_END: ephemeron_solution

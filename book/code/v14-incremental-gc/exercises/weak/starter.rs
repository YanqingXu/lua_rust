use crate::model::{Classifier, MarkHeap, Table, Value};

// ANCHOR: weak_edges_starter
pub fn classify_edges(table: &Table) -> Vec<Value> {
    // TODO 1: classify array/map edges by mode; metatable remains strong.
    let mut edges = table.array.clone();
    for &(key, value) in &table.map {
        edges.extend([key, value]);
    }
    if let Some(metatable) = table.metatable {
        edges.push(Value::Ref(metatable));
    }
    edges
}
// ANCHOR_END: weak_edges_starter

// ANCHOR: ephemeron_starter
pub fn propagate(heap: &mut MarkHeap, classify: Classifier) {
    // TODO 2: new strong marks can unlock values in another weak-key table.
    let candidates = heap.candidates();
    heap.record_round(&candidates);
    for value in candidates {
        heap.enqueue(value);
    }
    heap.drain(classify);
}
// ANCHOR_END: ephemeron_starter

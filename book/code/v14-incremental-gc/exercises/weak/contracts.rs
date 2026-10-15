use crate::model::{self, Classifier, MarkHeap, Mode, Propagator, Table, Value};
use crate::solution;

pub fn classifies_weak_edges(classify: Classifier) {
    let cases = [
        (Mode::Strong, false, vec!["W", "K", "V", "A", "M"], 1, 2),
        (Mode::Values, false, vec!["W", "K", "M"], 0, 1),
        (Mode::Keys, false, vec!["W", "A", "M"], 1, 1),
        (Mode::Keys, true, vec!["W", "K", "V", "A", "M"], 1, 2),
        (Mode::Both, false, vec!["W", "M"], 0, 1),
        (Mode::Both, true, vec!["W", "K", "M"], 0, 1),
    ];
    for (mode, root_key, labels, array_len, map_len) in cases {
        let mut heap = model::classification(mode);
        let roots = if root_key { vec![1, 2] } else { vec![1] };
        heap.collect(&roots, classify, solution::propagate);
        assert_eq!(
            heap.labels(),
            labels,
            "wrong strong edges for {mode:?}, root_key={root_key}"
        );
        assert_eq!(heap.tables[&1].array.len(), array_len);
        assert_eq!(heap.tables[&1].map.len(), map_len);
        assert!(
            heap.tables.contains_key(&5),
            "metatable is always a strong edge"
        );
    }
    let mut primitive_key = model::classification(Mode::Keys);
    primitive_key.tables.get_mut(&1).unwrap().map = vec![(Value::Atom, Value::Ref(3))];
    primitive_key.collect(&[1], classify, solution::propagate);
    assert!(
        primitive_key.tables.contains_key(&3),
        "primitive key is already live"
    );
    let mut cycle = model::unrooted_cycle();
    cycle.collect(&[1], classify, solution::propagate);
    assert_eq!(
        cycle.labels(),
        ["W"],
        "a value-to-key cycle cannot bootstrap marking"
    );
}

pub fn reaches_ephemeron_fixed_point(propagate: Propagator) {
    let mut chain = model::chain();
    chain.collect(&[1, 2, 3], solution::classify_edges, propagate);
    assert_eq!(chain.labels(), ["W1", "W2", "K1", "V1", "K2", "V2"]);
    assert_eq!(chain.tables[&2].map, [(Value::Ref(5), Value::Ref(6))]);
    let rounds = chain
        .trace
        .iter()
        .filter(|line| line.starts_with("candidates="))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        rounds,
        [
            "candidates=[\"V1\"]",
            "candidates=[\"V2\"]",
            "candidates=[]"
        ]
    );
    chain.collect(&[1, 2], solution::classify_edges, propagate);
    assert_eq!(
        chain.labels(),
        ["W1", "W2"],
        "ephemeron propagation is not permanent rooting"
    );
    assert!(chain.tables[&1].map.is_empty() && chain.tables[&2].map.is_empty());
    let mut cycle = model::unrooted_cycle();
    cycle.collect(&[1], solution::classify_edges, propagate);
    assert_eq!(cycle.labels(), ["W"]);
    assert!(cycle.tables[&1].map.is_empty());
    for productive_rounds in [1, 4, 16] {
        check_long_chain(productive_rounds, propagate);
    }
}

fn check_long_chain(length: usize, propagate: Propagator) {
    let mut tables = Vec::new();
    let mut roots = (1..=length).collect::<Vec<_>>();
    roots.push(length + 1); // Only K1, alongside all W tables, is an external root.
    for index in 0..length {
        let key_id = length + 1 + index * 2;
        let value_id = key_id + 1;
        let mut weak = Table::new("W", Mode::Keys);
        weak.map.push((Value::Ref(key_id), Value::Ref(value_id)));
        let mut value = Table::new("V", Mode::Strong);
        if index + 1 < length {
            value.array.push(Value::Ref(key_id + 2));
        }
        tables.extend([
            (index + 1, weak),
            (key_id, Table::new("K", Mode::Strong)),
            (value_id, value),
        ]);
    }
    let mut heap = MarkHeap::new(tables);
    heap.collect(&roots, solution::classify_edges, propagate);
    assert_eq!(
        heap.tables.len(),
        length * 3,
        "chain needs {length} productive rounds"
    );
    let productive_rounds = heap
        .trace
        .iter()
        .filter(|line| line.starts_with("candidates=") && line.as_str() != "candidates=[]")
        .count();
    assert_eq!(
        productive_rounds, length,
        "do not hardcode two propagation rounds"
    );
    assert_eq!(heap.trace.last().map(String::as_str), Some("candidates=[]"));
}

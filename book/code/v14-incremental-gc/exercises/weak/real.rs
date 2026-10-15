//! Real Heap probes: only configure graphs/modes between complete collection cycles.
use lua_core::{ByteString, Heap, TableRef, Value};

#[derive(Debug, Eq, PartialEq)]
pub struct WeakReport {
    pub key_accessible: bool,
    pub value_accessible: bool,
    pub array_value_accessible: bool,
    pub metatable_accessible: bool,
    pub map_entry: bool,
    pub array_entry: bool,
}

fn set_mode(heap: &mut Heap, metatable: TableRef, mode: &[u8]) {
    heap.table_set(
        metatable,
        Value::String(ByteString::new(b"__mode".as_slice())),
        Value::String(ByteString::new(mode)),
    )
    .unwrap();
}

fn weak_table(heap: &mut Heap, metatable: TableRef) -> TableRef {
    let table = heap.allocate_table().unwrap();
    heap.table_set_metatable(table, Some(metatable)).unwrap();
    table
}

fn accessible(heap: &Heap, table: TableRef) -> bool {
    heap.table_array_len(table).is_ok()
}

pub fn mode_report(mode: &[u8], root_key: bool) -> WeakReport {
    let mut heap = Heap::new();
    let metatable = heap.allocate_table().unwrap();
    set_mode(&mut heap, metatable, mode);
    let weak = weak_table(&mut heap, metatable);
    let key = heap.allocate_table().unwrap();
    let value = heap.allocate_table().unwrap();
    let array_value = heap.allocate_table().unwrap();
    heap.table_set(weak, Value::Table(key), Value::Table(value))
        .unwrap();
    heap.table_set(weak, Value::Number(1.0), Value::Table(array_value))
        .unwrap();
    let mut roots = vec![Value::Table(weak)];
    if root_key {
        roots.push(Value::Table(key));
    }
    heap.collect(&roots, &[]).unwrap();
    let array_entry = heap.table_get(weak, &Value::Number(1.0)).unwrap();
    if mode.contains(&b'v') {
        assert_eq!(array_entry, Value::Nil, "weak array must be cleared to nil");
        assert_eq!(heap.table_array_len(weak), Ok(0));
    } else {
        assert_eq!(array_entry, Value::Table(array_value));
        assert_eq!(heap.table_array_len(weak), Ok(1));
    }
    let mut cursor = Value::Nil;
    let mut map_entry = false;
    for _ in 0..4 {
        let Some((next, _)) = heap.table_next_pair(weak, &cursor).unwrap() else {
            break;
        };
        map_entry |= next == Value::Table(key);
        cursor = next;
    }
    WeakReport {
        key_accessible: accessible(&heap, key),
        value_accessible: accessible(&heap, value),
        array_value_accessible: accessible(&heap, array_value),
        metatable_accessible: accessible(&heap, metatable),
        map_entry,
        array_entry: array_entry == Value::Table(array_value),
    }
}

// ANCHOR: real_ephemeron_chain
pub fn cross_table_chain() -> (bool, bool) {
    let mut heap = Heap::new();
    let metatable = heap.allocate_table().unwrap();
    set_mode(&mut heap, metatable, b"k");
    let w1 = weak_table(&mut heap, metatable);
    let w2 = weak_table(&mut heap, metatable);
    let k1 = heap.allocate_table().unwrap();
    let v1 = heap.allocate_table().unwrap();
    let k2 = heap.allocate_table().unwrap();
    let v2 = heap.allocate_table().unwrap();
    heap.table_set(w1, Value::Table(k1), Value::Table(v1))
        .unwrap();
    heap.table_set(v1, Value::Number(1.0), Value::Table(k2))
        .unwrap();
    heap.table_set(w2, Value::Table(k2), Value::Table(v2))
        .unwrap();
    heap.collect(&[Value::Table(w1), Value::Table(w2), Value::Table(k1)], &[])
        .unwrap();
    let chain_live = [k1, v1, k2, v2]
        .iter()
        .all(|&table| accessible(&heap, table))
        && heap.table_get(w1, &Value::Table(k1)).unwrap() == Value::Table(v1)
        && heap.table_get(w2, &Value::Table(k2)).unwrap() == Value::Table(v2);
    heap.collect(&[Value::Table(w1), Value::Table(w2)], &[])
        .unwrap();
    let released = [k1, v1, k2, v2]
        .iter()
        .all(|&table| !accessible(&heap, table))
        && heap.table_next_pair(w1, &Value::Nil).unwrap().is_none()
        && heap.table_next_pair(w2, &Value::Nil).unwrap().is_none();
    (chain_live, released)
}
// ANCHOR_END: real_ephemeron_chain

pub fn unrooted_key_value_cycle() -> (bool, bool) {
    let mut heap = Heap::new();
    let metatable = heap.allocate_table().unwrap();
    set_mode(&mut heap, metatable, b"k");
    let weak = weak_table(&mut heap, metatable);
    let key = heap.allocate_table().unwrap();
    let value = heap.allocate_table().unwrap();
    heap.table_set(weak, Value::Table(key), Value::Table(value))
        .unwrap();
    heap.table_set(key, Value::Number(1.0), Value::Table(value))
        .unwrap();
    heap.table_set(value, Value::Number(1.0), Value::Table(key))
        .unwrap();
    heap.collect(&[Value::Table(weak)], &[]).unwrap();
    let both_reclaimed = !accessible(&heap, key) && !accessible(&heap, value);
    let entry_removed = heap.table_next_pair(weak, &Value::Nil).unwrap().is_none();
    (both_reclaimed, entry_removed)
}

#[derive(Debug, Eq, PartialEq)]
pub struct TransitionReport {
    pub weak_with_external_roots: bool,
    pub strong_keeps_both: bool,
    pub weak_again_reclaims_both: bool,
}

pub fn shared_mode_between_cycles() -> TransitionReport {
    let mut heap = Heap::new();
    let metatable = heap.allocate_table().unwrap();
    set_mode(&mut heap, metatable, b"v");
    let w1 = weak_table(&mut heap, metatable);
    let w2 = weak_table(&mut heap, metatable);
    let v1 = heap.allocate_table().unwrap();
    let v2 = heap.allocate_table().unwrap();
    heap.table_set(w1, Value::Number(1.0), Value::Table(v1))
        .unwrap();
    heap.table_set(w2, Value::Number(1.0), Value::Table(v2))
        .unwrap();
    heap.collect(
        &[
            Value::Table(w1),
            Value::Table(w2),
            Value::Table(v1),
            Value::Table(v2),
        ],
        &[],
    )
    .unwrap();
    let weak_with_external_roots = accessible(&heap, v1)
        && accessible(&heap, v2)
        && heap.table_get(w1, &Value::Number(1.0)).unwrap() == Value::Table(v1)
        && heap.table_get(w2, &Value::Number(1.0)).unwrap() == Value::Table(v2);
    set_mode(&mut heap, metatable, b"");
    heap.collect(&[Value::Table(w1), Value::Table(w2)], &[])
        .unwrap();
    let strong_keeps_both = accessible(&heap, v1)
        && accessible(&heap, v2)
        && heap.table_get(w1, &Value::Number(1.0)).unwrap() == Value::Table(v1)
        && heap.table_get(w2, &Value::Number(1.0)).unwrap() == Value::Table(v2);
    set_mode(&mut heap, metatable, b"v");
    heap.collect(&[Value::Table(w1), Value::Table(w2)], &[])
        .unwrap();
    let weak_again_reclaims_both = !accessible(&heap, v1)
        && !accessible(&heap, v2)
        && heap.table_get(w1, &Value::Number(1.0)).unwrap() == Value::Nil
        && heap.table_get(w2, &Value::Number(1.0)).unwrap() == Value::Nil
        && heap.table_array_len(w1) == Ok(0)
        && heap.table_array_len(w2) == Ok(0);
    TransitionReport {
        weak_with_external_roots,
        strong_keeps_both,
        weak_again_reclaims_both,
    }
}

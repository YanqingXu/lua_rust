//! Only marking and weak cleanup: no incremental mutation, allocation or finalizers.
use std::collections::{BTreeMap, BTreeSet};

pub type Id = usize;
pub type Classifier = fn(&Table) -> Vec<Value>;
pub type Propagator = fn(&mut MarkHeap, Classifier);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Nil,
    Atom,
    Ref(Id),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Strong,
    Values,
    Keys,
    Both,
}

impl Mode {
    pub fn weak_keys(self) -> bool {
        matches!(self, Self::Keys | Self::Both)
    }

    pub fn weak_values(self) -> bool {
        matches!(self, Self::Values | Self::Both)
    }
}

pub struct Table {
    pub mode: Mode,
    pub array: Vec<Value>,
    pub map: Vec<(Value, Value)>,
    pub metatable: Option<Id>,
    pub label: &'static str,
}

impl Table {
    pub fn new(label: &'static str, mode: Mode) -> Self {
        Self {
            mode,
            array: Vec::new(),
            map: Vec::new(),
            metatable: None,
            label,
        }
    }
}

pub struct MarkHeap {
    pub tables: BTreeMap<Id, Table>,
    marked: BTreeSet<Id>,
    gray: Vec<Id>,
    pub trace: Vec<String>,
}

impl MarkHeap {
    pub fn new(tables: impl IntoIterator<Item = (Id, Table)>) -> Self {
        Self {
            tables: tables.into_iter().collect(),
            marked: BTreeSet::new(),
            gray: Vec::new(),
            trace: Vec::new(),
        }
    }

    pub fn is_marked(&self, value: Value) -> bool {
        match value {
            Value::Ref(id) => self.marked.contains(&id),
            Value::Nil | Value::Atom => true,
        }
    }

    pub fn enqueue(&mut self, value: Value) {
        if let Value::Ref(id) = value {
            assert!(
                self.tables.contains_key(&id),
                "model input has a stale handle"
            );
            if !self.marked.contains(&id) {
                self.gray.push(id);
            }
        }
    }

    pub fn drain(&mut self, classify: Classifier) {
        while let Some(id) = self.gray.pop() {
            if !self.marked.insert(id) {
                continue;
            }
            let edges = classify(&self.tables[&id]);
            self.trace.push(format!("scan {}", self.tables[&id].label));
            for edge in edges {
                self.enqueue(edge);
            }
        }
    }

    // ANCHOR: ephemeron_candidates
    pub fn candidates(&self) -> Vec<Value> {
        let mut values = Vec::new();
        for (id, table) in &self.tables {
            if !self.marked.contains(id) || table.mode != Mode::Keys {
                continue;
            }
            for &(key, value) in &table.map {
                if self.is_marked(key) && !self.is_marked(value) {
                    values.push(value);
                }
            }
        }
        values
    }
    // ANCHOR_END: ephemeron_candidates

    pub fn record_round(&mut self, candidates: &[Value]) {
        let labels = candidates
            .iter()
            .filter_map(|value| match value {
                Value::Ref(id) => Some(self.tables[id].label),
                Value::Nil | Value::Atom => None,
            })
            .collect::<Vec<_>>();
        self.trace.push(format!("candidates={labels:?}"));
    }

    pub fn collect(&mut self, roots: &[Id], classify: Classifier, propagate: Propagator) {
        self.marked.clear();
        self.gray.clear();
        self.trace.clear();
        for &root in roots {
            self.enqueue(Value::Ref(root));
        }
        self.drain(classify);
        propagate(self, classify);
        let marked = &self.marked;
        let live = |value: Value| match value {
            Value::Ref(id) => marked.contains(&id),
            Value::Nil | Value::Atom => true,
        };
        for table in self.tables.values_mut() {
            if table.mode.weak_values() {
                for value in &mut table.array {
                    if !live(*value) {
                        *value = Value::Nil;
                    }
                }
                while table.array.last() == Some(&Value::Nil) {
                    table.array.pop();
                }
            }
            table.map.retain(|&(key, value)| {
                (!table.mode.weak_keys() || live(key)) && (!table.mode.weak_values() || live(value))
            });
        }
        self.tables.retain(|id, _| marked.contains(id));
    }

    pub fn labels(&self) -> Vec<&'static str> {
        self.tables.values().map(|table| table.label).collect()
    }
}

pub fn classification(mode: Mode) -> MarkHeap {
    let mut weak = Table::new("W", mode);
    weak.array.push(Value::Ref(4));
    weak.map = vec![(Value::Ref(2), Value::Ref(3)), (Value::Atom, Value::Atom)];
    weak.metatable = Some(5);
    MarkHeap::new([
        (1, weak),
        (2, Table::new("K", Mode::Strong)),
        (3, Table::new("V", Mode::Strong)),
        (4, Table::new("A", Mode::Strong)),
        (5, Table::new("M", Mode::Strong)),
    ])
}

// ANCHOR: ephemeron_chain_fixture
pub fn chain() -> MarkHeap {
    let mut w1 = Table::new("W1", Mode::Keys);
    w1.map.push((Value::Ref(3), Value::Ref(4)));
    let mut w2 = Table::new("W2", Mode::Keys);
    w2.map.push((Value::Ref(5), Value::Ref(6)));
    let mut v1 = Table::new("V1", Mode::Strong);
    v1.array.push(Value::Ref(5));
    MarkHeap::new([
        (1, w1),
        (2, w2),
        (3, Table::new("K1", Mode::Strong)),
        (4, v1),
        (5, Table::new("K2", Mode::Strong)),
        (6, Table::new("V2", Mode::Strong)),
    ])
}
// ANCHOR_END: ephemeron_chain_fixture

pub fn unrooted_cycle() -> MarkHeap {
    let mut weak = Table::new("W", Mode::Keys);
    weak.map.push((Value::Ref(2), Value::Ref(3)));
    let mut key = Table::new("K", Mode::Strong);
    key.array.push(Value::Ref(3));
    let mut value = Table::new("V", Mode::Strong);
    value.array.push(Value::Ref(2));
    MarkHeap::new([(1, weak), (2, key), (3, value)])
}

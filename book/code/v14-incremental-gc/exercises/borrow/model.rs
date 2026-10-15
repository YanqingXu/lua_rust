// This small graph models mark_one's borrow boundary, not the complete Heap.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Value {
    Primitive,
    Ref(usize),
}

struct Record {
    marked: bool,
    fields: Vec<Value>,
}

struct Heap {
    records: Vec<Record>,
    gray: Vec<usize>,
}

impl Heap {
    fn enqueue(&mut self, value: Value) {
        if let Value::Ref(id) = value {
            if !self.records[id].marked && !self.gray.contains(&id) {
                self.gray.push(id);
            }
        }
    }
}

fn main() {
    let fields = [
        vec![
            Value::Ref(1),
            Value::Primitive,
            Value::Ref(2),
            Value::Ref(1),
        ],
        vec![],
        vec![Value::Ref(1)],
        vec![Value::Ref(3)],
    ];
    let mut heap = Heap {
        records: fields
            .into_iter()
            .map(|fields| Record {
                marked: false,
                fields,
            })
            .collect(),
        gray: vec![],
    };
    heap.mark_one(0);
    assert_eq!(
        heap.gray,
        vec![1, 2],
        "both owned child edges must enter gray"
    );
    let marked: Vec<_> = heap.records.iter().map(|record| record.marked).collect();
    assert_eq!(marked, vec![true, false, false, false]);
    println!("after parent: gray=[1, 2] marked=[true, false, false, false]");
    while let Some(id) = heap.gray.pop() {
        heap.mark_one(id);
    }
    let marked: Vec<_> = heap.records.iter().map(|record| record.marked).collect();
    assert_eq!(marked, vec![true, true, true, false]);
    println!("after drain: gray=[] marked=[true, true, true, false]");
}

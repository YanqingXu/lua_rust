include!("model.rs");

impl Heap {
    // ANCHOR: long_borrow_before
    fn mark_one(&mut self, id: usize) {
        let record = &mut self.records[id];
        record.marked = true;
        for value in &record.fields {
            self.enqueue(value.clone());
        }
    }
    // ANCHOR_END: long_borrow_before
}

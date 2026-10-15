include!("model.rs");

impl Heap {
    // ANCHOR: short_borrow_starter
    fn mark_one(&mut self, id: usize) {
        // TODO: own the child values before calling another &mut self method.
        let record = &mut self.records[id];
        record.marked = true;
        for value in &record.fields {
            self.enqueue(value.clone());
        }
    }
    // ANCHOR_END: short_borrow_starter
}

include!("model.rs");

impl Heap {
    // ANCHOR: short_borrow_solution
    fn mark_one(&mut self, id: usize) {
        let children = {
            let record = &mut self.records[id];
            record.marked = true;
            record.fields.clone()
        }; // children owns its Values; no borrow of self.records escapes.
        for value in children {
            self.enqueue(value);
        }
    }
    // ANCHOR_END: short_borrow_solution
}

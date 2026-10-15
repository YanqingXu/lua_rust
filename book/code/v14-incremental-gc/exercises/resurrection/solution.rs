use crate::model::ResurrectionHeap;

// ANCHOR: resurrection_order_solution
pub fn finish_atomic(heap: &mut ResurrectionHeap) {
    heap.mark_pending_graph();
    heap.cleanup_weak();
}
// ANCHOR_END: resurrection_order_solution

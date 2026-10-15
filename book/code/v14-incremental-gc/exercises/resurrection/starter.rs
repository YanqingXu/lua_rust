use crate::model::ResurrectionHeap;

// ANCHOR: resurrection_order_starter
pub fn finish_atomic(heap: &mut ResurrectionHeap) {
    // TODO：weak 清理前，先完整标记已排队事件的 userdata/callback 及其强子图。
    heap.cleanup_weak();
    heap.mark_pending_graph();
}
// ANCHOR_END: resurrection_order_starter

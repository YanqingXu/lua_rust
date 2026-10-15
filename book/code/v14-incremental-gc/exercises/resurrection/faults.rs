//! 固定错误版本，不随学习者修改 starter 而改变。
use crate::model::ResurrectionHeap;

pub fn cleanup_too_early(heap: &mut ResurrectionHeap) {
    heap.cleanup_weak();
    heap.mark_pending_graph();
}

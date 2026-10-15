use crate::model::{Id, StrongHeap};
use lua_core::GcStep;

// ANCHOR: exercise_split
pub fn split_step(heap: &mut StrongHeap, roots: &[Id], _budget: usize) -> GcStep {
    // TODO 1：改为一次有预算的推进，不能首次调用就完成全部回收。
    heap.collect(roots)
}
// ANCHOR_END: exercise_split

// ANCHOR: exercise_resume
pub fn resume_step(heap: &mut StrongHeap, roots: &[Id], budget: usize) -> GcStep {
    // TODO 2：下一次必须接续同一周期，不能重置已保存的阶段/队列/游标。
    heap.step_restarting(roots, budget)
}
// ANCHOR_END: exercise_resume

// ANCHOR: exercise_barrier
pub fn connect(heap: &mut StrongHeap, parent: Id, child: Id) {
    // TODO 3：parent 已扫描后新增强边，必须保护尚未扫描的 child。
    heap.set_edge_without_barrier(parent, child);
}
// ANCHOR_END: exercise_barrier

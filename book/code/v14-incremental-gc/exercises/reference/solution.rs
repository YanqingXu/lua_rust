use crate::model::{Id, StrongHeap};
use lua_core::GcStep;

// ANCHOR: solution_split
pub fn split_step(heap: &mut StrongHeap, roots: &[Id], budget: usize) -> GcStep {
    heap.step(roots, budget)
}
// ANCHOR_END: solution_split

// ANCHOR: solution_resume
pub fn resume_step(heap: &mut StrongHeap, roots: &[Id], budget: usize) -> GcStep {
    heap.step(roots, budget)
}
// ANCHOR_END: solution_resume

// ANCHOR: solution_barrier
pub fn connect(heap: &mut StrongHeap, parent: Id, child: Id) {
    heap.set_edge(parent, child);
}
// ANCHOR_END: solution_barrier

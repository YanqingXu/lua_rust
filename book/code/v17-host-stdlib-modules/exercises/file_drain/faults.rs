use crate::model::Owner;

// ANCHOR: file_drain_fault
pub fn heap_only(owner: &mut Owner) {
    owner.shutdown_heap();
}
// ANCHOR_END: file_drain_fault

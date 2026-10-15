use crate::model::Owner;

// ANCHOR: file_drain_solution
pub fn shutdown(owner: &mut Owner) {
    owner.files.clear();
    owner.shutdown_heap();
}
// ANCHOR_END: file_drain_solution

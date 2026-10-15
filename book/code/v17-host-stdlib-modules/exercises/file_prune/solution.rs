use crate::model::FileModel;
use lua_core::GcStep;

// ANCHOR: file_prune_solution
pub fn prune(model: &mut FileModel, step: &GcStep) {
    if step.completed.is_some() {
        let heap = &model.heap;
        model
            .files
            .retain(|userdata, _| matches!(heap.is_live(*userdata), Ok(true)));
    }
}
// ANCHOR_END: file_prune_solution

use crate::model::FileModel;
use lua_core::GcStep;

// ANCHOR: file_prune_starter
pub fn prune(_model: &mut FileModel, _step: &GcStep) {
    // TODO: only at a completed cycle, retain entries whose userdata is live.
    // A live None is a closed-file tombstone and must remain in the map.
}
// ANCHOR_END: file_prune_starter

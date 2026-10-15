use crate::model::FileModel;
use lua_core::GcStep;

pub fn forget_prune(_model: &mut FileModel, _step: &GcStep) {}

pub fn prune_closed_instead_of_dead(model: &mut FileModel, step: &GcStep) {
    if step.completed.is_some() {
        model.files.retain(|_, file| file.is_some());
    }
}

use crate::model::{ChildYield, Coroutine, Frame, Status, TurnVm};

// ANCHOR: co_save_starter
pub fn save_frame(_coroutine: &mut Coroutine, scratch: &mut Option<Frame>, frame: Frame) {
    // TODO 1: each suspended coroutine must own its continuation.
    *scratch = Some(frame);
}
// ANCHOR_END: co_save_starter

// ANCHOR: co_resume_pc_starter
pub fn resume_pc(_frame: &Frame) -> usize {
    // TODO 2: use the saved next pc, rather than restarting entry side effects.
    0
}
// ANCHOR_END: co_resume_pc_starter

// ANCHOR: co_turn_starter
pub fn run_child(vm: &mut TurnVm) -> ChildYield {
    // TODO 3: save/restore parent owner and channels; isolate the child's output.
    vm.current = 1;
    vm.statuses[1] = Status::Running;
    vm.child_body();
    vm.statuses[1] = Status::Suspended;
    vm.current = 0;
    ChildYield {
        values: vm.yield_values.clone().unwrap_or_default(),
        frames: vm.yield_frames.clone(),
    }
}
// ANCHOR_END: co_turn_starter

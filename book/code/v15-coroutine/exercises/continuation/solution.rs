use crate::model::{ChildYield, Coroutine, Frame, Status, TurnVm};

// ANCHOR: co_save_solution
pub fn save_frame(coroutine: &mut Coroutine, _scratch: &mut Option<Frame>, frame: Frame) {
    coroutine.frame = Some(frame);
}
// ANCHOR_END: co_save_solution

// ANCHOR: co_resume_pc_solution
pub fn resume_pc(frame: &Frame) -> usize {
    frame.pc
}
// ANCHOR_END: co_resume_pc_solution

// ANCHOR: co_turn_solution
pub fn run_child(vm: &mut TurnVm) -> ChildYield {
    let parent = vm.current;
    vm.statuses[parent] = Status::Normal;
    let outer_values = vm.yield_values.take();
    let outer_frames = std::mem::take(&mut vm.yield_frames);
    vm.current = 1;
    vm.statuses[1] = Status::Running;
    vm.child_body();
    let yielded = ChildYield {
        values: vm.yield_values.take().unwrap_or_default(),
        frames: std::mem::take(&mut vm.yield_frames),
    };
    vm.statuses[1] = Status::Suspended;
    vm.current = parent;
    vm.statuses[parent] = Status::Running;
    vm.yield_values = outer_values;
    vm.yield_frames = outer_frames;
    yielded
}
// ANCHOR_END: co_turn_solution

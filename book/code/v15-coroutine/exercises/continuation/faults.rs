use crate::model::{ChildYield, Coroutine, Frame, Status, TurnVm};

pub fn shared_scratch(_coroutine: &mut Coroutine, scratch: &mut Option<Frame>, frame: Frame) {
    *scratch = Some(frame);
}

pub fn restart_entry(_frame: &Frame) -> usize {
    0
}

pub fn share_turn_channels(vm: &mut TurnVm) -> ChildYield {
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

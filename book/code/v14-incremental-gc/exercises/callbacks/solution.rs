use crate::model::{CallbackVm, Event, EventError, Handle};

// ANCHOR: callback_take_solution
pub fn take_batch(pending: &mut Vec<Event>) -> Vec<Event> {
    std::mem::take(pending)
}
// ANCHOR_END: callback_take_solution

// ANCHOR: callback_pin_solution
pub fn pin_batch(roots: &mut Vec<Handle>, events: &[Event]) {
    for event in events {
        roots.push(event.callback);
        roots.push(event.userdata);
    }
}
// ANCHOR_END: callback_pin_solution

// ANCHOR: callback_drain_solution
pub fn drain_batch(vm: &mut CallbackVm, batch: Vec<Event>) -> Result<(), EventError> {
    let root_base = vm.root_len();
    vm.pin_batch(&batch);
    let mut first_error = None;
    for event in batch {
        if let Err(error) = vm.invoke(event) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    vm.truncate_roots(root_base);
    first_error.map_or(Ok(()), Err)
}
// ANCHOR_END: callback_drain_solution

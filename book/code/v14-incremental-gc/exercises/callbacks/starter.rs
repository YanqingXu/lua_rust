use crate::model::{CallbackVm, Event, EventError, Handle};

// ANCHOR: callback_take_starter
#[allow(clippy::ptr_arg)] // 与修复版共享 take hook；修复需要移走整个 Vec。
pub fn take_batch(pending: &mut Vec<Event>) -> Vec<Event> {
    // TODO 1：克隆留下共享队列；回调重入还能看见正在处理的事件。
    pending.clone()
}
// ANCHOR_END: callback_take_starter

// ANCHOR: callback_pin_starter
pub fn pin_batch(_roots: &mut Vec<Handle>, _events: &[Event]) {
    // TODO 2：当前 invoke 的活动帧只保护当前事件；补齐全批临时 roots。
}
// ANCHOR_END: callback_pin_starter

// ANCHOR: callback_drain_starter
pub fn drain_batch(vm: &mut CallbackVm, batch: Vec<Event>) -> Result<(), EventError> {
    let root_base = vm.root_len();
    vm.pin_batch(&batch);
    for event in batch {
        // TODO 3：保存首错但继续执行，最终还原外层 roots 后再返回错误。
        vm.invoke(event)?;
    }
    vm.truncate_roots(root_base);
    Ok(())
}
// ANCHOR_END: callback_drain_starter

//! 固定错误重放，不随学习者修改 starter 而改变。
use crate::model::{CallbackVm, Event, EventError, Handle};

#[allow(clippy::ptr_arg)] // 同一个 hook 必须也能承载 mem::take 修复版。
pub fn clone_batch(pending: &mut Vec<Event>) -> Vec<Event> {
    pending.clone()
}

pub fn current_only(_roots: &mut Vec<Handle>, _events: &[Event]) {}

pub fn early_return(vm: &mut CallbackVm, batch: Vec<Event>) -> Result<(), EventError> {
    let base = vm.root_len();
    vm.pin_batch(&batch);
    for event in batch {
        vm.invoke(event)?;
    }
    vm.truncate_roots(base);
    Ok(())
}

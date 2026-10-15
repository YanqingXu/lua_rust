use crate::model::{Loader, RetryVm};
use lua_core::Value;
use lua_vm::VmError;

// ANCHOR: retry_attempt_starter
pub fn attempt(vm: &mut RetryVm, name: &str, loader: Loader) -> Result<Value, VmError> {
    // TODO：保存 loader Result，失败时只删除本 name，再返回原错误；成功走正常 finish。
    // 不恢复进入前的 false，不 clear 全部 loaded，也不回滚 loader 的其他副作用。
    crate::faults::question_mark(vm, name, loader)
}
// ANCHOR_END: retry_attempt_starter

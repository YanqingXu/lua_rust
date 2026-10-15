use crate::model::{Loader, RetryVm};
use lua_core::Value;
use lua_vm::VmError;

// ANCHOR: retry_question_mark_fault
pub fn question_mark(vm: &mut RetryVm, name: &str, loader: Loader) -> Result<Value, VmError> {
    let results = loader(vm, name)?;
    Ok(vm.finish(name, results))
}
// ANCHOR_END: retry_question_mark_fault

pub fn clear_all(vm: &mut RetryVm, name: &str, loader: Loader) -> Result<Value, VmError> {
    match loader(vm, name) {
        Ok(results) => Ok(vm.finish(name, results)),
        Err(error) => {
            vm.loaded.clear();
            Err(error)
        }
    }
}

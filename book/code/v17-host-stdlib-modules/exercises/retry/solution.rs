use crate::model::{Loader, RetryVm};
use lua_core::Value;
use lua_vm::VmError;

// ANCHOR: retry_attempt_solution
pub fn attempt(vm: &mut RetryVm, name: &str, loader: Loader) -> Result<Value, VmError> {
    let result = loader(vm, name);
    match result {
        Ok(results) => Ok(vm.finish(name, results)),
        Err(error) => {
            vm.loaded.remove(name);
            Err(error)
        }
    }
}
// ANCHOR_END: retry_attempt_solution

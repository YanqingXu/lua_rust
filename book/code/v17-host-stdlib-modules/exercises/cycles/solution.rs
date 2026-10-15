use crate::model::{CycleVm, LoadError, Loader, Name};
use lua_core::Value;

// ANCHOR: cycle_publish_solution
pub fn run_loader(vm: &mut CycleVm, name: Name, loader: Loader) -> Result<Value, LoadError> {
    vm.publish(name, Value::Boolean(true))?;
    let returned = loader(vm, name)?;
    vm.publish(name, returned.clone())?;
    Ok(returned)
}
// ANCHOR_END: cycle_publish_solution

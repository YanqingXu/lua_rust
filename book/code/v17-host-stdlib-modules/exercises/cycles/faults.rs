use crate::model::{CycleVm, LoadError, Loader, Name};
use lua_core::Value;

pub fn publish_too_late(vm: &mut CycleVm, name: Name, loader: Loader) -> Result<Value, LoadError> {
    let returned = loader(vm, name)?;
    vm.publish(name, Value::Boolean(true))?;
    vm.publish(name, returned.clone())?;
    Ok(returned)
}

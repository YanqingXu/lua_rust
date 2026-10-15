use crate::model::{CycleVm, LoadError, Loader, Name};
use lua_core::Value;

// ANCHOR: cycle_publish_starter
pub fn run_loader(vm: &mut CycleVm, name: Name, loader: Loader) -> Result<Value, LoadError> {
    // TODO：在任何 loader 重入前发布 true；正常 loader 结果再覆盖为最终 exports。
    let returned = loader(vm, name)?;
    vm.publish(name, Value::Boolean(true))?;
    vm.publish(name, returned.clone())?;
    Ok(returned)
}
// ANCHOR_END: cycle_publish_starter

use crate::model::{BoundaryError, BoundaryVm, Callback};

// ANCHOR: boundary_invoke_starter
pub fn invoke_boundary(vm: &mut BoundaryVm, callback: Callback) -> Result<i64, BoundaryError> {
    // TODO: retain Result until depth has been restored on both success and error.
    vm.depth += 1;
    vm.record("enter");
    let value = callback(vm)?;
    vm.depth -= 1;
    vm.record("restore");
    Ok(value)
}
// ANCHOR_END: boundary_invoke_starter

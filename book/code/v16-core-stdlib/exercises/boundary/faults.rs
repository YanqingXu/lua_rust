use crate::model::{BoundaryError, BoundaryVm, Callback};

pub fn early_question_mark(vm: &mut BoundaryVm, callback: Callback) -> Result<i64, BoundaryError> {
    vm.depth += 1;
    vm.record("enter");
    let value = callback(vm)?;
    vm.depth -= 1;
    vm.record("restore");
    Ok(value)
}

// ANCHOR: boundary_boolean_fault
pub fn depth_as_boolean(vm: &mut BoundaryVm, callback: Callback) -> Result<i64, BoundaryError> {
    vm.depth = 1;
    vm.record("enter bool");
    let result = callback(vm);
    vm.depth = 0;
    vm.record("clear bool");
    result
}
// ANCHOR_END: boundary_boolean_fault

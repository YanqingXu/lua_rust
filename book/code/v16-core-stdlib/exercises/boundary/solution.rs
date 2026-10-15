use crate::model::{BoundaryError, BoundaryVm, Callback};

// ANCHOR: boundary_invoke_solution
pub fn invoke_boundary(vm: &mut BoundaryVm, callback: Callback) -> Result<i64, BoundaryError> {
    let baseline = vm.depth;
    vm.depth += 1;
    vm.record("enter");
    let result = callback(vm);
    vm.depth = baseline;
    vm.record("restore");
    result
}
// ANCHOR_END: boundary_invoke_solution

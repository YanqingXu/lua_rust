use crate::model::{Operation, SortError, SortVm};
use lua_core::Value;

// ANCHOR: sort_snapshot_scope_solution
pub fn with_snapshot(
    vm: &mut SortVm,
    values: &mut [Value],
    operation: Operation,
) -> Result<(), SortError> {
    let root_base = vm.temporary_roots.len();
    vm.temporary_roots.extend_from_slice(values);
    let result = operation(vm, values);
    vm.temporary_roots.truncate(root_base);
    result
}
// ANCHOR_END: sort_snapshot_scope_solution

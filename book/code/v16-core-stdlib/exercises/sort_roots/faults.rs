use crate::model::{Operation, SortError, SortVm};
use lua_core::Value;

pub fn current_pair_only(
    vm: &mut SortVm,
    values: &mut [Value],
    operation: Operation,
) -> Result<(), SortError> {
    operation(vm, values)
}

pub fn early_question_mark(
    vm: &mut SortVm,
    values: &mut [Value],
    operation: Operation,
) -> Result<(), SortError> {
    let base = vm.temporary_roots.len();
    vm.temporary_roots.extend_from_slice(values);
    operation(vm, values)?;
    vm.temporary_roots.truncate(base);
    Ok(())
}

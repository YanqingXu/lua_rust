use crate::model::{Operation, SortError, SortVm};
use lua_core::Value;

// ANCHOR: sort_snapshot_scope_starter
pub fn with_snapshot(
    vm: &mut SortVm,
    values: &mut [Value],
    operation: Operation,
) -> Result<(), SortError> {
    // TODO：保护全 snapshot，保留外层 roots；成功/Err 都恢复进入时长度后再返回。
    operation(vm, values)
}
// ANCHOR_END: sort_snapshot_scope_starter

use crate::model::Environment;
use lua_core::{Heap, HeapError};

// ANCHOR: globals_install_starter
pub fn install(heap: &mut Heap) -> Result<Environment, HeapError> {
    // TODO：全局访问与初始 _G 必须使用同一 Heap 表，不能另维护 HashMap 快照。
    Environment::snapshot(heap)
}
// ANCHOR_END: globals_install_starter

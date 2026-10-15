use lua_core::{HeapError, Proto, TableRef, Value};
use lua_vm::Vm;

pub fn pin(_vm: &mut Vm, _key: Value, _value: Value) -> Result<(), HeapError> {
    // TODO：让宿主 Value 成为 registry 中的可达值。
    Ok(())
}

pub fn unpin(_vm: &mut Vm, _key: Value) -> Result<(), HeapError> {
    // TODO：结束宿主租约，删除相同 registry key 的值。
    Ok(())
}

pub fn restore_scope_exit(_original: &Proto, _faulty: &mut Proto) {
    // TODO：从 original 恢复被故障注入替换的 Close；保留指令数量和跳转位置。
}

pub fn primitive_roots(_metatables: &[TableRef]) -> Vec<Value> {
    // TODO：把每个 primitive metatable handle 转为可追踪的 root。
    Vec::new()
}

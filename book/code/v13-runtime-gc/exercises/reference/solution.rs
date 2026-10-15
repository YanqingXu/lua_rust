use lua_core::{HeapError, Opcode, Proto, TableRef, Value};
use lua_vm::Vm;

pub fn pin(vm: &mut Vm, key: Value, value: Value) -> Result<(), HeapError> {
    vm.registry_set(key, value)
}

pub fn unpin(vm: &mut Vm, key: Value) -> Result<(), HeapError> {
    vm.registry_set(key, Value::Nil)
}

pub fn restore_scope_exit(original: &Proto, faulty: &mut Proto) {
    for (source, target) in original.code.iter().zip(&mut faulty.code) {
        if source.opcode() == Ok(Opcode::Close) {
            *target = *source;
        }
    }
}

pub fn primitive_roots(metatables: &[TableRef]) -> Vec<Value> {
    metatables.iter().copied().map(Value::Table).collect()
}

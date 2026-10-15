use crate::model::{self, Machine};
use lua_core::{Instruction, StackSlot, Value};

pub fn record_after(machine: &mut Machine, instruction: Instruction) -> Result<(), String> {
    machine.execute(instruction)?;
    machine.record(instruction)
}

// ANCHOR: bytecode_trace_alias_fault
pub fn alias_history() -> Result<Vec<Vec<Value>>, String> {
    let mut machine = Machine::new();
    let mut history: Vec<Vec<StackSlot>> = vec![];
    for instruction in machine.proto.code.clone() {
        history.push(machine.registers.clone()); // 只克隆 Rc，仍指向活动寄存器。
        machine.execute(instruction)?;
    }
    history.iter().map(|slots| model::values(slots)).collect()
}
// ANCHOR_END: bytecode_trace_alias_fault

use crate::model::Machine;
use lua_core::Instruction;

// ANCHOR: bytecode_trace_starter
pub fn dispatch(machine: &mut Machine, instruction: Instruction) -> Result<(), String> {
    // TODO：当前 opcode 改寄存器之前，保存它真正的输入状态。
    machine.execute(instruction)?;
    machine.record(instruction)
}
// ANCHOR_END: bytecode_trace_starter

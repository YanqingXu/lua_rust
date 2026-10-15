use crate::model::Machine;
use lua_core::Instruction;

// ANCHOR: bytecode_trace_solution
pub fn dispatch(machine: &mut Machine, instruction: Instruction) -> Result<(), String> {
    machine.record(instruction)?;
    machine.execute(instruction)
}
// ANCHOR_END: bytecode_trace_solution

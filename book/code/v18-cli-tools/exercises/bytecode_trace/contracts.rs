use crate::model::{self, Dispatch};
use lua_core::{Opcode, Value};
use lua_vm::VmTraceEvent;

// ANCHOR: bytecode_trace_contract
pub fn assert_arithmetic_before(events: &[VmTraceEvent]) {
    assert_eq!(events.len(), 4);
    let expected = [
        vec![Value::Nil, Value::Nil],
        vec![Value::Number(40.0), Value::Nil],
        vec![Value::Number(40.0), Value::Number(2.0)],
        vec![Value::Number(42.0), Value::Number(2.0)],
    ];
    let opcodes = [Opcode::LoadK, Opcode::LoadK, Opcode::Add, Opcode::Return];
    for (pc, event) in events.iter().enumerate() {
        assert_eq!(event.pc, pc);
        assert_eq!(event.line, pc + 10);
        assert_eq!(event.frame_depth, 0); // 实际 call_stack.len()，顶层不是 1。
        assert_eq!(event.function, "trace-before");
        assert_eq!(event.opcode, opcodes[pc]);
        assert_eq!(event.registers, expected[pc], "pc={pc} 必须是执行前状态");
    }
}

pub fn records_current_registers_before_dispatch(dispatch: Dispatch) {
    assert_arithmetic_before(&model::run(dispatch).unwrap());
}
// ANCHOR_END: bytecode_trace_contract

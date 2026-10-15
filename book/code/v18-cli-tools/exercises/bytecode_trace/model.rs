//! 固定四条 Instruction 的寄存器实验；只解释 LoadK/Add/Return，不实现另一套 VM。
use lua_core::{Instruction, Opcode, Proto, StackSlot, Value};
use lua_vm::VmTraceEvent;
use std::cell::RefCell;
use std::rc::Rc;

pub type Dispatch = fn(&mut Machine, Instruction) -> Result<(), String>;

pub fn arithmetic_proto() -> Proto {
    Proto {
        constants: vec![Value::Number(40.0), Value::Number(2.0)],
        code: vec![
            Instruction::abx(Opcode::LoadK, 0, 0),
            Instruction::abx(Opcode::LoadK, 1, 1),
            Instruction::abc(Opcode::Add, 0, 0, 1),
            Instruction::abc(Opcode::Return, 0, 2, 0),
        ],
        lines: vec![10, 11, 12, 13],
        max_stack_size: 2,
        protos: vec![],
        num_params: 0,
        is_vararg: false,
        upvalues: vec![],
        debug_name: "trace-before".into(),
    }
}

pub fn values(slots: &[StackSlot]) -> Result<Vec<Value>, String> {
    slots
        .iter()
        .map(|slot| {
            slot.try_borrow()
                .map(|value| value.clone())
                .map_err(|error| error.to_string())
        })
        .collect()
}

pub struct Machine {
    pub proto: Proto,
    pub registers: Vec<StackSlot>,
    pub events: Vec<VmTraceEvent>,
    pub pc: usize,
}

impl Machine {
    pub fn new() -> Self {
        Self {
            proto: arithmetic_proto(),
            registers: (0..2).map(|_| Rc::new(RefCell::new(Value::Nil))).collect(),
            events: vec![],
            pc: 0,
        }
    }

    // ANCHOR: bytecode_trace_record
    pub fn record(&mut self, instruction: Instruction) -> Result<(), String> {
        self.events.push(VmTraceEvent {
            frame_depth: 0,
            function: self.proto.debug_name.clone(),
            pc: self.pc,
            line: self.proto.lines[self.pc],
            opcode: instruction
                .opcode()
                .map_err(|raw| format!("invalid opcode {raw}"))?,
            registers: values(&self.registers)?, // 克隆 Value，不保存活动 StackSlot 别名。
        });
        Ok(())
    }
    // ANCHOR_END: bytecode_trace_record

    // ANCHOR: bytecode_trace_dispatch
    pub fn execute(&mut self, instruction: Instruction) -> Result<(), String> {
        let value = match instruction.opcode().map_err(|raw| raw.to_string())? {
            Opcode::LoadK => self.proto.constants[instruction.bx() as usize].clone(),
            Opcode::Add => {
                let left = self.registers[instruction.b() as usize].borrow().clone();
                let right = self.registers[instruction.c() as usize].borrow().clone();
                let (Value::Number(left), Value::Number(right)) = (left, right) else {
                    return Err("fixture Add requires two numbers".into());
                };
                Value::Number(left + right)
            }
            Opcode::Return => return Ok(()), // 固定末条；本实验不建返回窗口/调用帧。
            _ => return Err("opcode outside this three-opcode fixture".into()),
        };
        *self.registers[usize::from(instruction.a())].borrow_mut() = value;
        Ok(())
    }
    // ANCHOR_END: bytecode_trace_dispatch
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run(dispatch: Dispatch) -> Result<Vec<VmTraceEvent>, String> {
    let mut machine = Machine::new();
    for (pc, instruction) in machine.proto.code.clone().into_iter().enumerate() {
        machine.pc = pc;
        dispatch(&mut machine, instruction)?;
    }
    Ok(machine.events)
}

use lua_core::{ByteString, FunctionRef, Opcode, Proto, TableRef, Value};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmError {
    pub pc: usize,
    pub line: usize,
    pub message: String,
    pub traceback: Vec<String>,
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {} (pc {}): {}",
            self.line, self.pc, self.message
        )?;
        for frame in &self.traceback {
            write!(formatter, "\n\t{frame}")?;
        }
        Ok(())
    }
}

impl std::error::Error for VmError {}

pub struct Vm {
    instruction_limit: usize,
    globals: HashMap<ByteString, Value>,
    max_call_depth: usize,
    call_stack: Vec<CallInfo>,
}

#[derive(Clone, Debug)]
struct CallInfo {
    source: String,
    line: usize,
    base: usize,
    top: usize,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Self {
            instruction_limit: 1_000_000,
            globals: HashMap::new(),
            max_call_depth: 32,
            call_stack: Vec::new(),
        }
    }

    pub fn with_instruction_limit(instruction_limit: usize) -> Self {
        Self {
            instruction_limit,
            globals: HashMap::new(),
            max_call_depth: 32,
            call_stack: Vec::new(),
        }
    }

    pub fn with_call_depth_limit(mut self, max_call_depth: usize) -> Self {
        self.max_call_depth = max_call_depth;
        self
    }

    // ANCHOR: dispatch
    pub fn execute(&mut self, proto: &Proto) -> Result<Value, VmError> {
        self.call_stack.clear();
        self.execute_proto(proto, &[])
    }

    // ANCHOR: call_frame
    fn execute_proto(&mut self, proto: &Proto, arguments: &[Value]) -> Result<Value, VmError> {
        proto.validate_shape().map_err(|message| VmError {
            pc: 0,
            line: 0,
            message: message.to_owned(),
            traceback: Vec::new(),
        })?;
        let mut registers = vec![Value::Nil; usize::from(proto.max_stack_size)];
        for (index, argument) in arguments
            .iter()
            .take(usize::from(proto.num_params))
            .enumerate()
        {
            if let Some(register) = registers.get_mut(index) {
                *register = argument.clone();
            }
        }
        let mut pc = 0_usize;
        let mut steps = 0_usize;

        while pc < proto.code.len() {
            if steps >= self.instruction_limit {
                return Err(self.error(proto, pc, "instruction limit exceeded"));
            }
            steps += 1;
            let instruction = proto.code[pc];
            let opcode = instruction
                .opcode()
                .map_err(|raw| self.error(proto, pc, format!("invalid opcode {raw}")))?;
            let mut next_pc = pc + 1;

            match opcode {
                Opcode::LoadK => {
                    let value = proto
                        .constants
                        .get(instruction.bx() as usize)
                        .cloned()
                        .ok_or_else(|| self.error(proto, pc, "constant index is out of bounds"))?;
                    self.write(&mut registers, instruction.a(), value, proto, pc)?;
                }
                // ANCHOR: globals
                Opcode::GetGlobal => {
                    let name = proto
                        .constants
                        .get(instruction.bx() as usize)
                        .ok_or_else(|| {
                            self.error(proto, pc, "name constant index is out of bounds")
                        })?;
                    let Value::String(name) = name else {
                        return Err(self.error(proto, pc, "GETGLOBAL constant is not a string"));
                    };
                    let value = self.globals.get(name).cloned().unwrap_or(Value::Nil);
                    self.write(&mut registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::SetGlobal => {
                    let name = proto
                        .constants
                        .get(instruction.bx() as usize)
                        .ok_or_else(|| {
                            self.error(proto, pc, "name constant index is out of bounds")
                        })?;
                    let Value::String(name) = name else {
                        return Err(self.error(proto, pc, "SETGLOBAL constant is not a string"));
                    };
                    let value = self
                        .read(&registers, u16::from(instruction.a()), proto, pc)?
                        .clone();
                    if value == Value::Nil {
                        self.globals.remove(name);
                    } else {
                        self.globals.insert(name.clone(), value);
                    }
                }
                // ANCHOR_END: globals
                Opcode::Move => {
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    self.write(&mut registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::Neg | Opcode::Not | Opcode::Len => {
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    let result = match opcode {
                        Opcode::Not => Value::Boolean(!value.is_truthy()),
                        Opcode::Neg => match value {
                            Value::Number(value) => Value::Number(-value),
                            other => {
                                return Err(self.error(
                                    proto,
                                    pc,
                                    format!("cannot negate {}, expected number", other.type_name()),
                                ));
                            }
                        },
                        Opcode::Len => match value {
                            Value::String(value) => Value::Number(value.as_bytes().len() as f64),
                            Value::Table(value) => Value::Number(
                                value
                                    .array_len()
                                    .map_err(|error| self.error(proto, pc, error.to_string()))?
                                    as f64,
                            ),
                            other => {
                                return Err(self.error(
                                    proto,
                                    pc,
                                    format!(
                                        "cannot get length of {}, expected string",
                                        other.type_name()
                                    ),
                                ));
                            }
                        },
                        _ => unreachable!(),
                    };
                    self.write(&mut registers, instruction.a(), result, proto, pc)?;
                }
                Opcode::JumpIfFalse | Opcode::JumpIfTrue => {
                    let condition = self
                        .read(&registers, u16::from(instruction.a()), proto, pc)?
                        .is_truthy();
                    let should_jump = if opcode == Opcode::JumpIfFalse {
                        !condition
                    } else {
                        condition
                    };
                    if should_jump {
                        let target = pc as i64 + 1 + i64::from(instruction.sbx());
                        if !(0..proto.code.len() as i64).contains(&target) {
                            return Err(self.error(proto, pc, "jump target is out of bounds"));
                        }
                        next_pc = target as usize;
                    }
                }
                // ANCHOR: loop_vm
                Opcode::Jump => {
                    next_pc = self.jump_target(proto, pc, instruction.sbx())?;
                }
                Opcode::ForPrep => {
                    let base = instruction.a();
                    let limit_register = self.offset_register(base, 1, proto, pc)?;
                    let step_register = self.offset_register(base, 2, proto, pc)?;
                    let initial = self.number_register(&registers, base, proto, pc)?;
                    let limit = self.number_register(&registers, limit_register, proto, pc)?;
                    let step = self.number_register(&registers, step_register, proto, pc)?;
                    if step == 0.0 {
                        return Err(self.error(proto, pc, "numeric for step must not be zero"));
                    }
                    if initial.is_nan() || limit.is_nan() || step.is_nan() {
                        return Err(self.error(proto, pc, "numeric for values must not be NaN"));
                    }
                    self.write(
                        &mut registers,
                        base,
                        Value::Number(initial - step),
                        proto,
                        pc,
                    )?;
                    next_pc = self.jump_target(proto, pc, instruction.sbx())?;
                }
                Opcode::ForLoop => {
                    let base = instruction.a();
                    let limit_register = self.offset_register(base, 1, proto, pc)?;
                    let step_register = self.offset_register(base, 2, proto, pc)?;
                    let visible_register = self.offset_register(base, 3, proto, pc)?;
                    let index = self.number_register(&registers, base, proto, pc)?;
                    let limit = self.number_register(&registers, limit_register, proto, pc)?;
                    let step = self.number_register(&registers, step_register, proto, pc)?;
                    let next = index + step;
                    let continues = if step > 0.0 {
                        next <= limit
                    } else {
                        next >= limit
                    };
                    if continues {
                        self.write(&mut registers, base, Value::Number(next), proto, pc)?;
                        self.write(
                            &mut registers,
                            visible_register,
                            Value::Number(next),
                            proto,
                            pc,
                        )?;
                        next_pc = self.jump_target(proto, pc, instruction.sbx())?;
                    }
                }
                // ANCHOR_END: loop_vm
                Opcode::Return => {
                    return self
                        .read(&registers, u16::from(instruction.a()), proto, pc)
                        .cloned();
                }
                Opcode::Closure => {
                    let nested = proto
                        .protos
                        .get(instruction.bx() as usize)
                        .cloned()
                        .ok_or_else(|| {
                            self.error(proto, pc, "nested proto index is out of bounds")
                        })?;
                    self.write(
                        &mut registers,
                        instruction.a(),
                        Value::Function(FunctionRef::new(nested)),
                        proto,
                        pc,
                    )?;
                }
                Opcode::Call => {
                    let function = self
                        .read(&registers, u16::from(instruction.a()), proto, pc)?
                        .clone();
                    let Value::Function(function) = function else {
                        return Err(self.error(
                            proto,
                            pc,
                            format!("attempt to call a {} value", function.type_name()),
                        ));
                    };
                    if self.call_stack.len() >= self.max_call_depth {
                        return Err(self.error(proto, pc, "call depth limit exceeded"));
                    }
                    let first = usize::from(instruction.b());
                    let count = usize::from(instruction.c());
                    let mut arguments = Vec::with_capacity(count);
                    for index in 0..count {
                        arguments.push(registers.get(first + index).cloned().ok_or_else(|| {
                            self.error(proto, pc, "call argument window is out of bounds")
                        })?);
                    }
                    self.call_stack.push(CallInfo {
                        source: proto.debug_name.clone(),
                        line: proto.lines.get(pc).copied().unwrap_or(0),
                        base: first,
                        top: first + count,
                    });
                    let call_result = self.execute_proto(function.proto(), &arguments);
                    self.call_stack.pop();
                    let result = call_result?;
                    self.write(&mut registers, instruction.a(), result, proto, pc)?;
                }
                // ANCHOR: table_vm
                Opcode::NewTable => {
                    self.write(
                        &mut registers,
                        instruction.a(),
                        Value::Table(TableRef::new()),
                        proto,
                        pc,
                    )?;
                }
                Opcode::GetTable => {
                    let table = self.table_register(&registers, instruction.b(), proto, pc)?;
                    let key = self.read(&registers, instruction.c(), proto, pc)?;
                    let value = table
                        .get(key)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(&mut registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::SetTable => {
                    let table =
                        self.table_register(&registers, u16::from(instruction.a()), proto, pc)?;
                    let key = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    let value = self.read(&registers, instruction.c(), proto, pc)?.clone();
                    table
                        .set(key, value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                Opcode::SetList => {
                    let table =
                        self.table_register(&registers, u16::from(instruction.a()), proto, pc)?;
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    table
                        .set(Value::Number(f64::from(instruction.c())), value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                // ANCHOR_END: table_vm
                Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::Div
                | Opcode::Mod
                | Opcode::Pow
                | Opcode::Concat
                | Opcode::Eq
                | Opcode::Ne
                | Opcode::Lt
                | Opcode::Le
                | Opcode::Gt
                | Opcode::Ge => {
                    let left = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    let right = self.read(&registers, instruction.c(), proto, pc)?.clone();
                    let result = self.binary(opcode, left, right, proto, pc)?;
                    self.write(&mut registers, instruction.a(), result, proto, pc)?;
                }
            }
            pc = next_pc;
        }
        Err(self.error(proto, pc.saturating_sub(1), "proto ended without RETURN"))
    }
    // ANCHOR_END: call_frame
    // ANCHOR_END: dispatch

    fn binary(
        &self,
        opcode: Opcode,
        left: Value,
        right: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<Value, VmError> {
        match opcode {
            Opcode::Eq | Opcode::Ne => {
                let equal = left == right;
                Ok(Value::Boolean(if opcode == Opcode::Eq {
                    equal
                } else {
                    !equal
                }))
            }
            Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge => {
                let ordering = compare(&left, &right).ok_or_else(|| {
                    self.error(
                        proto,
                        pc,
                        format!(
                            "cannot compare {} with {}",
                            left.type_name(),
                            right.type_name()
                        ),
                    )
                })?;
                Ok(Value::Boolean(match opcode {
                    Opcode::Lt => ordering == Ordering::Less,
                    Opcode::Le => ordering != Ordering::Greater,
                    Opcode::Gt => ordering == Ordering::Greater,
                    Opcode::Ge => ordering != Ordering::Less,
                    _ => unreachable!(),
                }))
            }
            Opcode::Concat => {
                let left_type = left.type_name();
                let right_type = right.type_name();
                let mut bytes = concat_bytes(left).ok_or_else(|| {
                    self.error(proto, pc, format!("cannot concatenate {left_type}"))
                })?;
                let right = concat_bytes(right).ok_or_else(|| {
                    self.error(proto, pc, format!("cannot concatenate {right_type}"))
                })?;
                bytes.extend(right);
                Ok(Value::String(ByteString::new(bytes)))
            }
            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Mod | Opcode::Pow => {
                let (Value::Number(left), Value::Number(right)) = (&left, &right) else {
                    return Err(self.error(
                        proto,
                        pc,
                        format!(
                            "arithmetic needs numbers, got {} and {}",
                            left.type_name(),
                            right.type_name()
                        ),
                    ));
                };
                Ok(Value::Number(match opcode {
                    Opcode::Add => left + right,
                    Opcode::Sub => left - right,
                    Opcode::Mul => left * right,
                    Opcode::Div => left / right,
                    Opcode::Mod => left - (left / right).floor() * right,
                    Opcode::Pow => left.powf(*right),
                    _ => unreachable!(),
                }))
            }
            _ => unreachable!("caller only passes eager binary opcodes"),
        }
    }

    fn read<'registers>(
        &self,
        registers: &'registers [Value],
        register: u16,
        proto: &Proto,
        pc: usize,
    ) -> Result<&'registers Value, VmError> {
        registers
            .get(usize::from(register))
            .ok_or_else(|| self.error(proto, pc, format!("register {register} is out of bounds")))
    }

    fn number_register(
        &self,
        registers: &[Value],
        register: u8,
        proto: &Proto,
        pc: usize,
    ) -> Result<f64, VmError> {
        match self.read(registers, u16::from(register), proto, pc)? {
            Value::Number(value) => Ok(*value),
            other => Err(self.error(
                proto,
                pc,
                format!("numeric for expected number, got {}", other.type_name()),
            )),
        }
    }

    fn table_register(
        &self,
        registers: &[Value],
        register: u16,
        proto: &Proto,
        pc: usize,
    ) -> Result<TableRef, VmError> {
        match self.read(registers, register, proto, pc)? {
            Value::Table(value) => Ok(value.clone()),
            other => Err(self.error(
                proto,
                pc,
                format!("attempt to index a {} value", other.type_name()),
            )),
        }
    }

    fn offset_register(
        &self,
        base: u8,
        offset: u8,
        proto: &Proto,
        pc: usize,
    ) -> Result<u8, VmError> {
        base.checked_add(offset)
            .ok_or_else(|| self.error(proto, pc, "numeric for register window overflows"))
    }

    fn jump_target(&self, proto: &Proto, pc: usize, offset: i32) -> Result<usize, VmError> {
        let target = pc as i64 + 1 + i64::from(offset);
        if !(0..proto.code.len() as i64).contains(&target) {
            return Err(self.error(proto, pc, "jump target is out of bounds"));
        }
        Ok(target as usize)
    }

    fn write(
        &self,
        registers: &mut [Value],
        register: u8,
        value: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<(), VmError> {
        let slot = registers.get_mut(usize::from(register)).ok_or_else(|| {
            self.error(proto, pc, format!("register {register} is out of bounds"))
        })?;
        *slot = value;
        Ok(())
    }

    fn error(&self, proto: &Proto, pc: usize, message: impl Into<String>) -> VmError {
        VmError {
            pc,
            line: proto.lines.get(pc).copied().unwrap_or(0),
            message: message.into(),
            traceback: self
                .call_stack
                .iter()
                .rev()
                .map(|frame| {
                    format!(
                        "called from line {} in {} (base={}, top={})",
                        frame.line, frame.source, frame.base, frame.top
                    )
                })
                .collect(),
        }
    }
}

fn compare(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.partial_cmp(right),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn concat_bytes(value: Value) -> Option<Vec<u8>> {
    match value {
        Value::String(value) => Some(value.as_bytes().to_vec()),
        Value::Number(value) => Some(format!("{value}").into_bytes()),
        Value::Nil | Value::Boolean(_) | Value::Table(_) | Value::Function(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Vm;
    use lua_core::{Instruction, Opcode, Proto, Value};

    #[test]
    fn malformed_register_is_rejected_instead_of_panicking() {
        let proto = Proto {
            constants: vec![],
            code: vec![Instruction::abc(Opcode::Return, 7, 0, 0)],
            lines: vec![12],
            max_stack_size: 1,
            protos: vec![],
            num_params: 0,
            debug_name: "malformed-register".to_owned(),
        };
        let error = Vm::new().execute(&proto).unwrap_err();
        assert_eq!(error.line, 12);
        assert!(error.message.contains("register 7"));
    }

    #[test]
    fn malformed_constant_is_rejected() {
        let proto = Proto {
            constants: vec![Value::Number(1.0)],
            code: vec![
                Instruction::abx(Opcode::LoadK, 0, 99),
                Instruction::abc(Opcode::Return, 0, 0, 0),
            ],
            lines: vec![1, 1],
            max_stack_size: 1,
            protos: vec![],
            num_params: 0,
            debug_name: "malformed-constant".to_owned(),
        };
        assert!(Vm::new()
            .execute(&proto)
            .unwrap_err()
            .message
            .contains("constant index"));
    }
}

use lua_core::{ByteString, Opcode, Proto, Value};
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmError {
    pub pc: usize,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {} (pc {}): {}",
            self.line, self.pc, self.message
        )
    }
}

impl std::error::Error for VmError {}

pub struct Vm {
    instruction_limit: usize,
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
        }
    }

    pub fn with_instruction_limit(instruction_limit: usize) -> Self {
        Self { instruction_limit }
    }

    // ANCHOR: dispatch
    pub fn execute(&self, proto: &Proto) -> Result<Value, VmError> {
        proto.validate_shape().map_err(|message| VmError {
            pc: 0,
            line: 0,
            message: message.to_owned(),
        })?;
        let mut registers = vec![Value::Nil; usize::from(proto.max_stack_size)];
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
                Opcode::GetName => {
                    let name = proto
                        .constants
                        .get(instruction.bx() as usize)
                        .ok_or_else(|| {
                            self.error(proto, pc, "name constant index is out of bounds")
                        })?;
                    let Value::String(name) = name else {
                        return Err(self.error(proto, pc, "GETNAME constant is not a string"));
                    };
                    return Err(self.error(proto, pc, format!("undefined name '{name}'")));
                }
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
                Opcode::Return => {
                    return self
                        .read(&registers, u16::from(instruction.a()), proto, pc)
                        .cloned();
                }
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
        Value::Nil | Value::Boolean(_) => None,
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
        };
        assert!(Vm::new()
            .execute(&proto)
            .unwrap_err()
            .message
            .contains("constant index"));
    }
}

use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteString(Box<[u8]>);

impl ByteString {
    pub fn new(bytes: impl Into<Box<[u8]>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for ByteString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in self.as_bytes() {
            for escaped in std::ascii::escape_default(byte) {
                write!(formatter, "{}", char::from(escaped))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(ByteString),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
        }
    }

    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Boolean(false))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nil => formatter.write_str("nil"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "\"{value}\""),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Opcode {
    Move,
    LoadK,
    GetGlobal,
    Neg,
    Not,
    Len,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    JumpIfFalse,
    JumpIfTrue,
    Return,
    SetGlobal,
}

impl TryFrom<u8> for Opcode {
    type Error = u8;

    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        match raw {
            0 => Ok(Self::Move),
            1 => Ok(Self::LoadK),
            2 => Ok(Self::GetGlobal),
            3 => Ok(Self::Neg),
            4 => Ok(Self::Not),
            5 => Ok(Self::Len),
            6 => Ok(Self::Add),
            7 => Ok(Self::Sub),
            8 => Ok(Self::Mul),
            9 => Ok(Self::Div),
            10 => Ok(Self::Mod),
            11 => Ok(Self::Pow),
            12 => Ok(Self::Concat),
            13 => Ok(Self::Eq),
            14 => Ok(Self::Ne),
            15 => Ok(Self::Lt),
            16 => Ok(Self::Le),
            17 => Ok(Self::Gt),
            18 => Ok(Self::Ge),
            19 => Ok(Self::JumpIfFalse),
            20 => Ok(Self::JumpIfTrue),
            21 => Ok(Self::Return),
            22 => Ok(Self::SetGlobal),
            other => Err(other),
        }
    }
}

const SIZE_OP: u32 = 6;
const SIZE_A: u32 = 8;
const SIZE_C: u32 = 9;
const POS_A: u32 = SIZE_OP;
const POS_C: u32 = POS_A + SIZE_A;
const POS_B: u32 = POS_C + SIZE_C;
const MAX_A: u32 = (1 << SIZE_A) - 1;
const MAX_BC: u32 = (1 << SIZE_C) - 1;
const MAX_BX: u32 = (1 << (SIZE_C * 2)) - 1;
pub const MAX_SBX: i32 = (MAX_BX as i32) / 2;

// ANCHOR: instruction
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction(u32);

impl Instruction {
    pub fn abc(opcode: Opcode, a: u8, b: u16, c: u16) -> Self {
        assert!(u32::from(b) <= MAX_BC && u32::from(c) <= MAX_BC);
        Self(
            opcode as u32
                | (u32::from(a) << POS_A)
                | (u32::from(c) << POS_C)
                | (u32::from(b) << POS_B),
        )
    }

    pub fn abx(opcode: Opcode, a: u8, bx: u32) -> Self {
        assert!(bx <= MAX_BX);
        Self(opcode as u32 | (u32::from(a) << POS_A) | (bx << POS_C))
    }

    pub fn asbx(opcode: Opcode, a: u8, sbx: i32) -> Self {
        assert!((-MAX_SBX..=MAX_SBX).contains(&sbx));
        Self::abx(opcode, a, (sbx + MAX_SBX) as u32)
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub fn opcode(self) -> Result<Opcode, u8> {
        Opcode::try_from((self.0 & ((1 << SIZE_OP) - 1)) as u8)
    }

    pub fn a(self) -> u8 {
        ((self.0 >> POS_A) & MAX_A) as u8
    }

    pub fn b(self) -> u16 {
        ((self.0 >> POS_B) & MAX_BC) as u16
    }

    pub fn c(self) -> u16 {
        ((self.0 >> POS_C) & MAX_BC) as u16
    }

    pub fn bx(self) -> u32 {
        (self.0 >> POS_C) & MAX_BX
    }

    pub fn sbx(self) -> i32 {
        self.bx() as i32 - MAX_SBX
    }
}
// ANCHOR_END: instruction

// ANCHOR: proto
#[derive(Clone, Debug, PartialEq)]
pub struct Proto {
    pub constants: Vec<Value>,
    pub code: Vec<Instruction>,
    pub lines: Vec<usize>,
    pub max_stack_size: u8,
}
// ANCHOR_END: proto

impl Proto {
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if self.code.len() != self.lines.len() {
            return Err("instruction and line tables have different lengths");
        }
        if self.max_stack_size == 0 {
            return Err("proto needs at least one register");
        }
        Ok(())
    }
}

pub fn disassemble(proto: &Proto) -> Result<String, String> {
    let mut output = String::new();
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        let operands = match opcode {
            Opcode::LoadK | Opcode::GetGlobal | Opcode::SetGlobal => {
                format!("{} {}", instruction.a(), instruction.bx())
            }
            Opcode::JumpIfFalse | Opcode::JumpIfTrue => {
                format!("{} {:+}", instruction.a(), instruction.sbx())
            }
            Opcode::Neg | Opcode::Not | Opcode::Len | Opcode::Move | Opcode::Return => {
                format!("{} {}", instruction.a(), instruction.b())
            }
            _ => format!(
                "{} {} {}",
                instruction.a(),
                instruction.b(),
                instruction.c()
            ),
        };
        let opcode_name = format!("{opcode:?}");
        output.push_str(&format!(
            "{pc:04} [{:>3}] {opcode_name:<12} {operands}\n",
            proto.lines[pc]
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{Instruction, Opcode, MAX_SBX};

    #[test]
    fn abc_encoding_round_trips() {
        let instruction = Instruction::abc(Opcode::Add, 17, 301, 511);
        let decoded = Instruction::from_raw(instruction.raw());
        assert_eq!(decoded.opcode(), Ok(Opcode::Add));
        assert_eq!((decoded.a(), decoded.b(), decoded.c()), (17, 301, 511));
    }

    #[test]
    fn signed_jump_encoding_round_trips() {
        for offset in [-MAX_SBX, -7, 0, 42, MAX_SBX] {
            let instruction = Instruction::asbx(Opcode::JumpIfFalse, 9, offset);
            assert_eq!(instruction.sbx(), offset);
            assert_eq!(instruction.a(), 9);
        }
    }
}

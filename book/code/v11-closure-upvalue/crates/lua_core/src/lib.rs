use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

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

// ANCHOR: table_storage
#[derive(Clone)]
pub struct TableRef(Rc<RefCell<Table>>);

impl TableRef {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Table::new())))
    }

    pub fn get(&self, key: &Value) -> Result<Value, TableError> {
        self.0
            .try_borrow()
            .map_err(|_| TableError::BorrowConflict)?
            .get(key)
    }

    pub fn set(&self, key: Value, value: Value) -> Result<(), TableError> {
        self.0
            .try_borrow_mut()
            .map_err(|_| TableError::BorrowConflict)?
            .set(key, value)
    }

    pub fn array_len(&self) -> Result<usize, TableError> {
        Ok(self
            .0
            .try_borrow()
            .map_err(|_| TableError::BorrowConflict)?
            .array
            .len())
    }

    pub fn next_pair(&self, after: &Value) -> Result<Option<(Value, Value)>, TableError> {
        self.0
            .try_borrow()
            .map_err(|_| TableError::BorrowConflict)?
            .next_pair(after)
    }

    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    pub fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }
}

impl Default for TableRef {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TableRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "TableRef(0x{:x})", self.identity())
    }
}

impl PartialEq for TableRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TableRef {}

impl Hash for TableRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TableKey {
    Boolean(bool),
    Number(u64),
    String(ByteString),
    Table(TableRef),
    Function(FunctionRef),
    NativeFunction(NativeFunction),
}

impl TableKey {
    fn from_value(value: &Value) -> Result<Self, TableError> {
        match value {
            Value::Nil => Err(TableError::NilKey),
            Value::Boolean(value) => Ok(Self::Boolean(*value)),
            Value::Number(value) if value.is_nan() => Err(TableError::NanKey),
            Value::Number(value) => {
                let normalized = if *value == 0.0 { 0.0 } else { *value };
                Ok(Self::Number(normalized.to_bits()))
            }
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Table(value) => Ok(Self::Table(value.clone())),
            Value::Function(value) => Ok(Self::Function(value.clone())),
            Value::NativeFunction(value) => Ok(Self::NativeFunction(*value)),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Boolean(value) => Value::Boolean(*value),
            Self::Number(bits) => Value::Number(f64::from_bits(*bits)),
            Self::String(value) => Value::String(value.clone()),
            Self::Table(value) => Value::Table(value.clone()),
            Self::Function(value) => Value::Function(value.clone()),
            Self::NativeFunction(value) => Value::NativeFunction(*value),
        }
    }
}

#[derive(Debug)]
struct Table {
    array: Vec<Value>,
    map: HashMap<TableKey, Value>,
}

impl Table {
    fn new() -> Self {
        Self {
            array: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn get(&self, key: &Value) -> Result<Value, TableError> {
        if let Some(index) = array_index(key) {
            if let Some(value) = self.array.get(index - 1) {
                return Ok(value.clone());
            }
        }
        let key = TableKey::from_value(key)?;
        Ok(self.map.get(&key).cloned().unwrap_or(Value::Nil))
    }

    fn set(&mut self, key: Value, value: Value) -> Result<(), TableError> {
        let table_key = TableKey::from_value(&key)?;
        if let Some(index) = array_index(&key) {
            if index <= self.array.len() {
                self.array[index - 1] = value;
                self.trim_array();
                return Ok(());
            }
            if index == self.array.len() + 1 && value != Value::Nil {
                self.array.push(value);
                self.map.remove(&table_key);
                self.promote_integer_tail();
                return Ok(());
            }
        }
        if value == Value::Nil {
            self.map.remove(&table_key);
        } else {
            self.map.insert(table_key, value);
        }
        Ok(())
    }

    fn trim_array(&mut self) {
        while self.array.last() == Some(&Value::Nil) {
            self.array.pop();
        }
    }

    fn promote_integer_tail(&mut self) {
        loop {
            let next = (self.array.len() + 1) as f64;
            let key = TableKey::Number(next.to_bits());
            let Some(value) = self.map.remove(&key) else {
                break;
            };
            self.array.push(value);
        }
    }

    fn next_pair(&self, after: &Value) -> Result<Option<(Value, Value)>, TableError> {
        let mut entries = self
            .array
            .iter()
            .enumerate()
            .filter(|(_, value)| **value != Value::Nil)
            .map(|(index, value)| (Value::Number((index + 1) as f64), value.clone()))
            .collect::<Vec<_>>();
        entries.extend(
            self.map
                .iter()
                .map(|(key, value)| (key.to_value(), value.clone())),
        );
        if *after == Value::Nil {
            return Ok(entries.into_iter().next());
        }
        let Some(index) = entries.iter().position(|(key, _)| key == after) else {
            return Err(TableError::InvalidNextKey);
        };
        Ok(entries.into_iter().nth(index + 1))
    }
}

fn array_index(value: &Value) -> Option<usize> {
    let Value::Number(value) = value else {
        return None;
    };
    (*value >= 1.0 && value.is_finite() && value.fract() == 0.0 && *value <= usize::MAX as f64)
        .then_some(*value as usize)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableError {
    NilKey,
    NanKey,
    BorrowConflict,
    InvalidNextKey,
}

impl fmt::Display for TableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NilKey => formatter.write_str("table index is nil"),
            Self::NanKey => formatter.write_str("table index is NaN"),
            Self::BorrowConflict => formatter.write_str("table is already mutably borrowed"),
            Self::InvalidNextKey => formatter.write_str("invalid key to 'next'"),
        }
    }
}

impl std::error::Error for TableError {}
// ANCHOR_END: table_storage

// ANCHOR: function_ref
pub type StackSlot = Rc<RefCell<Value>>;

#[derive(Clone)]
pub struct UpvalueRef(Rc<RefCell<UpvalueState>>);

#[derive(Clone, Debug)]
enum UpvalueState {
    Open(StackSlot),
    Closed(Value),
}

impl UpvalueRef {
    pub fn open(slot: StackSlot) -> Self {
        Self(Rc::new(RefCell::new(UpvalueState::Open(slot))))
    }

    pub fn get(&self) -> Value {
        match &*self.0.borrow() {
            UpvalueState::Open(slot) => slot.borrow().clone(),
            UpvalueState::Closed(value) => value.clone(),
        }
    }

    pub fn set(&self, value: Value) {
        match &mut *self.0.borrow_mut() {
            UpvalueState::Open(slot) => *slot.borrow_mut() = value,
            UpvalueState::Closed(current) => *current = value,
        }
    }

    pub fn close(&self) {
        let value = self.get();
        *self.0.borrow_mut() = UpvalueState::Closed(value);
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.0.borrow(), UpvalueState::Open(_))
    }

    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
}

impl fmt::Debug for UpvalueRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "UpvalueRef(0x{:x})", self.identity())
    }
}

impl PartialEq for UpvalueRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for UpvalueRef {}

#[derive(Clone)]
pub struct FunctionRef(Rc<Closure>);

#[derive(Debug)]
struct Closure {
    proto: Rc<Proto>,
    upvalues: Vec<UpvalueRef>,
}

impl FunctionRef {
    pub fn new(proto: Rc<Proto>) -> Self {
        Self::with_upvalues(proto, Vec::new())
    }

    pub fn with_upvalues(proto: Rc<Proto>, upvalues: Vec<UpvalueRef>) -> Self {
        Self(Rc::new(Closure { proto, upvalues }))
    }

    pub fn proto(&self) -> &Rc<Proto> {
        &self.0.proto
    }

    pub fn upvalues(&self) -> &[UpvalueRef] {
        &self.0.upvalues
    }

    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
}

impl fmt::Debug for FunctionRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "FunctionRef(0x{:x})", self.identity())
    }
}

impl PartialEq for FunctionRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for FunctionRef {}

impl Hash for FunctionRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}
// ANCHOR_END: function_ref

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NativeFunction {
    Next,
    Pairs,
    IPairs,
    IPairsAux,
}

impl NativeFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Pairs => "pairs",
            Self::IPairs => "ipairs",
            Self::IPairsAux => "ipairsaux",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(ByteString),
    Table(TableRef),
    Function(FunctionRef),
    NativeFunction(NativeFunction),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Boolean(left), Self::Boolean(right)) => left == right,
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Table(left), Self::Table(right)) => left == right,
            (Self::Function(left), Self::Function(right)) => left == right,
            (Self::NativeFunction(left), Self::NativeFunction(right)) => left == right,
            _ => false,
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
            Self::Function(_) => "function",
            Self::NativeFunction(_) => "function",
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
            Self::Table(value) => write!(formatter, "table: 0x{:x}", value.identity()),
            Self::Function(value) => write!(formatter, "function: 0x{:x}", value.identity()),
            Self::NativeFunction(value) => write!(formatter, "function: builtin:{}", value.name()),
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
    Jump,
    ForPrep,
    ForLoop,
    NewTable,
    GetTable,
    SetTable,
    SetList,
    Closure,
    Call,
    Vararg,
    TailCall,
    JumpIfNil,
    SetListOpen,
    GetUpval,
    SetUpval,
    Close,
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
            23 => Ok(Self::Jump),
            24 => Ok(Self::ForPrep),
            25 => Ok(Self::ForLoop),
            26 => Ok(Self::NewTable),
            27 => Ok(Self::GetTable),
            28 => Ok(Self::SetTable),
            29 => Ok(Self::SetList),
            30 => Ok(Self::Closure),
            31 => Ok(Self::Call),
            32 => Ok(Self::Vararg),
            33 => Ok(Self::TailCall),
            34 => Ok(Self::JumpIfNil),
            35 => Ok(Self::SetListOpen),
            36 => Ok(Self::GetUpval),
            37 => Ok(Self::SetUpval),
            38 => Ok(Self::Close),
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
    pub protos: Vec<Rc<Proto>>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub upvalues: Vec<UpvalueDesc>,
    pub debug_name: String,
}
// ANCHOR_END: proto

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpvalueDesc {
    pub name: ByteString,
    pub in_stack: bool,
    pub index: u8,
}

impl Proto {
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if self.code.len() != self.lines.len() {
            return Err("instruction and line tables have different lengths");
        }
        if self.max_stack_size == 0 {
            return Err("proto needs at least one register");
        }
        if self.num_params > self.max_stack_size {
            return Err("parameter count exceeds register window");
        }
        for nested in &self.protos {
            for upvalue in &nested.upvalues {
                if upvalue.in_stack && upvalue.index >= self.max_stack_size {
                    return Err("nested upvalue stack index is out of bounds");
                }
                if !upvalue.in_stack && usize::from(upvalue.index) >= self.upvalues.len() {
                    return Err("nested upvalue parent index is out of bounds");
                }
            }
            nested.validate_shape()?;
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
            Opcode::LoadK | Opcode::GetGlobal | Opcode::SetGlobal | Opcode::Closure => {
                format!("{} {}", instruction.a(), instruction.bx())
            }
            Opcode::JumpIfFalse
            | Opcode::JumpIfTrue
            | Opcode::JumpIfNil
            | Opcode::Jump
            | Opcode::ForPrep
            | Opcode::ForLoop => {
                format!("{} {:+}", instruction.a(), instruction.sbx())
            }
            Opcode::Neg
            | Opcode::Not
            | Opcode::Len
            | Opcode::Move
            | Opcode::Return
            | Opcode::Vararg
            | Opcode::GetUpval
            | Opcode::SetUpval
            | Opcode::Close => {
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
    use super::{
        ByteString, Instruction, Opcode, StackSlot, TableError, TableRef, UpvalueRef, Value,
        MAX_SBX,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

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

    #[test]
    fn closing_an_upvalue_detaches_it_from_the_stack_slot() {
        let slot: StackSlot = Rc::new(RefCell::new(Value::Number(1.0)));
        let upvalue = UpvalueRef::open(slot.clone());
        upvalue.set(Value::Number(2.0));
        assert_eq!(*slot.borrow(), Value::Number(2.0));
        upvalue.close();
        assert!(!upvalue.is_open());
        *slot.borrow_mut() = Value::Number(99.0);
        assert_eq!(upvalue.get(), Value::Number(2.0));
    }

    #[test]
    fn table_array_hash_and_deletion_rules_are_explicit() {
        let table = TableRef::new();
        table.set(Value::Number(1.0), Value::Number(10.0)).unwrap();
        table.set(Value::Number(3.0), Value::Number(30.0)).unwrap();
        table
            .set(
                Value::String(ByteString::new(&b"name"[..])),
                Value::String(ByteString::new(&b"lua"[..])),
            )
            .unwrap();
        assert_eq!(table.array_len(), Ok(1));
        assert_eq!(table.get(&Value::Number(3.0)), Ok(Value::Number(30.0)));
        table.set(Value::Number(2.0), Value::Number(20.0)).unwrap();
        assert_eq!(table.array_len(), Ok(3));
        table.set(Value::Number(3.0), Value::Nil).unwrap();
        assert_eq!(table.array_len(), Ok(2));
        assert_eq!(table.get(&Value::Number(3.0)), Ok(Value::Nil));
    }

    #[test]
    fn invalid_keys_are_rejected() {
        let table = TableRef::new();
        assert_eq!(
            table.set(Value::Nil, Value::Number(1.0)),
            Err(TableError::NilKey)
        );
        assert_eq!(
            table.set(Value::Number(f64::NAN), Value::Number(1.0)),
            Err(TableError::NanKey)
        );
    }

    #[test]
    fn handles_share_identity_and_make_cycles_visible() {
        let table = TableRef::new();
        let alias = table.clone();
        alias
            .set(
                Value::String(ByteString::new(&b"answer"[..])),
                Value::Number(42.0),
            )
            .unwrap();
        assert_eq!(
            table.get(&Value::String(ByteString::new(&b"answer"[..]))),
            Ok(Value::Number(42.0))
        );
        table
            .set(
                Value::String(ByteString::new(&b"self"[..])),
                Value::Table(table.clone()),
            )
            .unwrap();
        assert!(table.strong_count() >= 3);
    }
}

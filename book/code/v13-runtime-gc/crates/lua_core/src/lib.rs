use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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
pub enum TableObject {}
pub enum ClosureObject {}
pub enum UpvalueObject {}

pub struct GcRef<T> {
    heap_id: u64,
    object_id: u64,
    marker: PhantomData<fn() -> T>,
}

impl<T> GcRef<T> {
    fn new(heap_id: u64, object_id: u64) -> Self {
        Self {
            heap_id,
            object_id,
            marker: PhantomData,
        }
    }

    pub fn heap_id(self) -> u64 {
        self.heap_id
    }

    pub fn object_id(self) -> u64 {
        self.object_id
    }

    pub fn identity(self) -> u128 {
        (u128::from(self.heap_id) << 64) | u128::from(self.object_id)
    }
}

impl<T> Copy for GcRef<T> {}

impl<T> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for GcRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GcRef({}:{})", self.heap_id, self.object_id)
    }
}

impl<T> PartialEq for GcRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.heap_id == other.heap_id && self.object_id == other.object_id
    }
}

impl<T> Eq for GcRef<T> {}

impl<T> Hash for GcRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.heap_id.hash(state);
        self.object_id.hash(state);
    }
}

pub type TableRef = GcRef<TableObject>;
pub type FunctionRef = GcRef<ClosureObject>;
pub type UpvalueRef = GcRef<UpvalueObject>;

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
            Value::Table(value) => Ok(Self::Table(*value)),
            Value::Function(value) => Ok(Self::Function(*value)),
            Value::NativeFunction(value) => Ok(Self::NativeFunction(*value)),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Boolean(value) => Value::Boolean(*value),
            Self::Number(bits) => Value::Number(f64::from_bits(*bits)),
            Self::String(value) => Value::String(value.clone()),
            Self::Table(value) => Value::Table(*value),
            Self::Function(value) => Value::Function(*value),
            Self::NativeFunction(value) => Value::NativeFunction(*value),
        }
    }
}

#[derive(Clone, Debug)]
struct Table {
    array: Vec<Value>,
    map: HashMap<TableKey, Value>,
    metatable: Option<TableRef>,
}

impl Table {
    fn new() -> Self {
        Self {
            array: Vec::new(),
            map: HashMap::new(),
            metatable: None,
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
    InvalidNextKey,
}

impl fmt::Display for TableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NilKey => formatter.write_str("table index is nil"),
            Self::NanKey => formatter.write_str("table index is NaN"),
            Self::InvalidNextKey => formatter.write_str("invalid key to 'next'"),
        }
    }
}

impl std::error::Error for TableError {}
// ANCHOR_END: table_storage

// ANCHOR: function_ref
pub type StackSlot = Rc<RefCell<Value>>;

#[derive(Clone, Debug)]
enum UpvalueState {
    Open(StackSlot),
    Closed(Value),
}

#[derive(Clone, Debug)]
struct Closure {
    proto: Rc<Proto>,
    upvalues: Vec<UpvalueRef>,
}

#[derive(Clone, Debug)]
enum HeapObject {
    Table(Table),
    Closure(Closure),
    Upvalue(UpvalueState),
}

#[derive(Clone, Debug)]
struct HeapRecord {
    marked: bool,
    object: HeapObject,
}

static NEXT_HEAP_ID: AtomicU64 = AtomicU64::new(1);

// ANCHOR: heap_owner
#[derive(Debug)]
pub struct Heap {
    heap_id: u64,
    next_object_id: u64,
    objects: HashMap<u64, HeapRecord>,
    shutdown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GcStats {
    pub before: usize,
    pub collected: usize,
    pub remaining: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapError {
    ForeignHandle { expected: u64, actual: u64 },
    StaleHandle { object_id: u64 },
    WrongObjectKind { object_id: u64 },
    StackBorrowConflict,
    Shutdown,
    Table(TableError),
}

impl fmt::Display for HeapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignHandle { expected, actual } => {
                write!(
                    formatter,
                    "foreign GC handle from heap {actual}, expected heap {expected}"
                )
            }
            Self::StaleHandle { object_id } => {
                write!(formatter, "stale GC handle for object {object_id}")
            }
            Self::WrongObjectKind { object_id } => {
                write!(formatter, "GC handle {object_id} has the wrong object kind")
            }
            Self::StackBorrowConflict => formatter.write_str("stack slot is already borrowed"),
            Self::Shutdown => formatter.write_str("heap has already shut down"),
            Self::Table(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HeapError {}

impl From<TableError> for HeapError {
    fn from(error: TableError) -> Self {
        Self::Table(error)
    }
}

#[derive(Clone, Copy, Debug)]
enum TraceRef {
    Table(TableRef),
    Function(FunctionRef),
    Upvalue(UpvalueRef),
}

impl Heap {
    pub fn new() -> Self {
        Self {
            heap_id: NEXT_HEAP_ID.fetch_add(1, AtomicOrdering::Relaxed),
            next_object_id: 1,
            objects: HashMap::new(),
            shutdown: false,
        }
    }

    pub fn heap_id(&self) -> u64 {
        self.heap_id
    }

    pub fn live_count(&self) -> usize {
        self.objects.len()
    }

    pub fn allocate_table(&mut self) -> Result<TableRef, HeapError> {
        let object_id = self.allocate(HeapObject::Table(Table::new()))?;
        Ok(TableRef::new(self.heap_id, object_id))
    }

    pub fn allocate_function(&mut self, proto: Rc<Proto>) -> Result<FunctionRef, HeapError> {
        self.allocate_function_with_upvalues(proto, Vec::new())
    }

    pub fn allocate_function_with_upvalues(
        &mut self,
        proto: Rc<Proto>,
        upvalues: Vec<UpvalueRef>,
    ) -> Result<FunctionRef, HeapError> {
        self.validate_proto(&proto)?;
        for upvalue in &upvalues {
            self.validate(*upvalue)?;
        }
        let object_id = self.allocate(HeapObject::Closure(Closure { proto, upvalues }))?;
        Ok(FunctionRef::new(self.heap_id, object_id))
    }

    pub fn allocate_open_upvalue(&mut self, slot: StackSlot) -> Result<UpvalueRef, HeapError> {
        let object_id = self.allocate(HeapObject::Upvalue(UpvalueState::Open(slot)))?;
        Ok(UpvalueRef::new(self.heap_id, object_id))
    }

    pub fn table_get(&self, table: TableRef, key: &Value) -> Result<Value, HeapError> {
        self.validate_value(key)?;
        self.table(table)?.get(key).map_err(Into::into)
    }

    pub fn table_set(
        &mut self,
        table: TableRef,
        key: Value,
        value: Value,
    ) -> Result<(), HeapError> {
        self.validate_value(&key)?;
        self.validate_value(&value)?;
        self.table_mut(table)?.set(key, value).map_err(Into::into)
    }

    pub fn table_array_len(&self, table: TableRef) -> Result<usize, HeapError> {
        Ok(self.table(table)?.array.len())
    }

    pub fn table_metatable(&self, table: TableRef) -> Result<Option<TableRef>, HeapError> {
        Ok(self.table(table)?.metatable)
    }

    pub fn table_set_metatable(
        &mut self,
        table: TableRef,
        metatable: Option<TableRef>,
    ) -> Result<(), HeapError> {
        if let Some(metatable) = metatable {
            self.validate(metatable)?;
        }
        self.table_mut(table)?.metatable = metatable;
        Ok(())
    }

    pub fn table_next_pair(
        &self,
        table: TableRef,
        after: &Value,
    ) -> Result<Option<(Value, Value)>, HeapError> {
        self.table(table)?.next_pair(after).map_err(Into::into)
    }

    pub fn function_proto(&self, function: FunctionRef) -> Result<Rc<Proto>, HeapError> {
        Ok(self.closure(function)?.proto.clone())
    }

    pub fn function_upvalues(&self, function: FunctionRef) -> Result<Vec<UpvalueRef>, HeapError> {
        Ok(self.closure(function)?.upvalues.clone())
    }

    pub fn upvalue_get(&self, upvalue: UpvalueRef) -> Result<Value, HeapError> {
        match self.upvalue(upvalue)? {
            UpvalueState::Open(slot) => slot
                .try_borrow()
                .map(|value| value.clone())
                .map_err(|_| HeapError::StackBorrowConflict),
            UpvalueState::Closed(value) => Ok(value.clone()),
        }
    }

    pub fn upvalue_set(&mut self, upvalue: UpvalueRef, value: Value) -> Result<(), HeapError> {
        self.validate_value(&value)?;
        match self.upvalue_mut(upvalue)? {
            UpvalueState::Open(slot) => {
                *slot
                    .try_borrow_mut()
                    .map_err(|_| HeapError::StackBorrowConflict)? = value;
            }
            UpvalueState::Closed(current) => *current = value,
        }
        Ok(())
    }

    pub fn close_upvalue(&mut self, upvalue: UpvalueRef) -> Result<(), HeapError> {
        let value = self.upvalue_get(upvalue)?;
        *self.upvalue_mut(upvalue)? = UpvalueState::Closed(value);
        Ok(())
    }

    pub fn upvalue_is_open(&self, upvalue: UpvalueRef) -> Result<bool, HeapError> {
        Ok(matches!(self.upvalue(upvalue)?, UpvalueState::Open(_)))
    }

    pub fn is_live<T>(&self, handle: GcRef<T>) -> Result<bool, HeapError> {
        self.validate_heap(handle)?;
        Ok(self.objects.contains_key(&handle.object_id))
    }

    // ANCHOR: mark_sweep
    pub fn collect(
        &mut self,
        roots: &[Value],
        upvalue_roots: &[UpvalueRef],
    ) -> Result<GcStats, HeapError> {
        self.ensure_running()?;
        let before = self.objects.len();
        let result = self.mark_from(roots, upvalue_roots);
        if let Err(error) = result {
            for record in self.objects.values_mut() {
                record.marked = false;
            }
            return Err(error);
        }
        self.objects.retain(|_, record| {
            let keep = record.marked;
            record.marked = false;
            keep
        });
        let remaining = self.objects.len();
        Ok(GcStats {
            before,
            collected: before - remaining,
            remaining,
        })
    }
    // ANCHOR_END: mark_sweep

    pub fn shutdown(&mut self) -> usize {
        let released = self.objects.len();
        self.objects.clear();
        self.shutdown = true;
        released
    }

    fn allocate(&mut self, object: HeapObject) -> Result<u64, HeapError> {
        self.ensure_running()?;
        let object_id = self.next_object_id;
        self.next_object_id = self
            .next_object_id
            .checked_add(1)
            .expect("teaching heap exhausted its object ID space");
        self.objects.insert(
            object_id,
            HeapRecord {
                marked: false,
                object,
            },
        );
        Ok(object_id)
    }

    fn ensure_running(&self) -> Result<(), HeapError> {
        if self.shutdown {
            Err(HeapError::Shutdown)
        } else {
            Ok(())
        }
    }

    fn validate_heap<T>(&self, handle: GcRef<T>) -> Result<(), HeapError> {
        self.ensure_running()?;
        if handle.heap_id != self.heap_id {
            return Err(HeapError::ForeignHandle {
                expected: self.heap_id,
                actual: handle.heap_id,
            });
        }
        Ok(())
    }

    fn validate<T>(&self, handle: GcRef<T>) -> Result<(), HeapError> {
        self.validate_heap(handle)?;
        if self.objects.contains_key(&handle.object_id) {
            Ok(())
        } else {
            Err(HeapError::StaleHandle {
                object_id: handle.object_id,
            })
        }
    }

    fn validate_value(&self, value: &Value) -> Result<(), HeapError> {
        match value {
            Value::Table(table) => {
                self.table(*table)?;
            }
            Value::Function(function) => {
                self.closure(*function)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_proto(&self, proto: &Proto) -> Result<(), HeapError> {
        for value in &proto.constants {
            self.validate_value(value)?;
        }
        for nested in &proto.protos {
            self.validate_proto(nested)?;
        }
        Ok(())
    }

    fn record<T>(&self, handle: GcRef<T>) -> Result<&HeapRecord, HeapError> {
        self.validate(handle)?;
        Ok(&self.objects[&handle.object_id])
    }

    fn record_mut<T>(&mut self, handle: GcRef<T>) -> Result<&mut HeapRecord, HeapError> {
        self.validate(handle)?;
        Ok(self
            .objects
            .get_mut(&handle.object_id)
            .expect("validated object disappeared"))
    }

    fn table(&self, table: TableRef) -> Result<&Table, HeapError> {
        match &self.record(table)?.object {
            HeapObject::Table(table) => Ok(table),
            _ => Err(HeapError::WrongObjectKind {
                object_id: table.object_id,
            }),
        }
    }

    fn table_mut(&mut self, table: TableRef) -> Result<&mut Table, HeapError> {
        match &mut self.record_mut(table)?.object {
            HeapObject::Table(table) => Ok(table),
            _ => Err(HeapError::WrongObjectKind {
                object_id: table.object_id,
            }),
        }
    }

    fn closure(&self, function: FunctionRef) -> Result<&Closure, HeapError> {
        match &self.record(function)?.object {
            HeapObject::Closure(closure) => Ok(closure),
            _ => Err(HeapError::WrongObjectKind {
                object_id: function.object_id,
            }),
        }
    }

    fn upvalue(&self, upvalue: UpvalueRef) -> Result<&UpvalueState, HeapError> {
        match &self.record(upvalue)?.object {
            HeapObject::Upvalue(upvalue) => Ok(upvalue),
            _ => Err(HeapError::WrongObjectKind {
                object_id: upvalue.object_id,
            }),
        }
    }

    fn upvalue_mut(&mut self, upvalue: UpvalueRef) -> Result<&mut UpvalueState, HeapError> {
        match &mut self.record_mut(upvalue)?.object {
            HeapObject::Upvalue(upvalue) => Ok(upvalue),
            _ => Err(HeapError::WrongObjectKind {
                object_id: upvalue.object_id,
            }),
        }
    }

    fn mark_from(
        &mut self,
        roots: &[Value],
        upvalue_roots: &[UpvalueRef],
    ) -> Result<(), HeapError> {
        let mut values = roots.to_vec();
        let mut pending = upvalue_roots
            .iter()
            .copied()
            .map(TraceRef::Upvalue)
            .collect::<Vec<_>>();
        while !values.is_empty() || !pending.is_empty() {
            while let Some(value) = values.pop() {
                match value {
                    Value::Table(table) => pending.push(TraceRef::Table(table)),
                    Value::Function(function) => pending.push(TraceRef::Function(function)),
                    _ => {}
                }
            }
            let Some(reference) = pending.pop() else {
                continue;
            };
            let (object_id, expected_kind) = match reference {
                TraceRef::Table(handle) => {
                    self.validate(handle)?;
                    (handle.object_id, 0)
                }
                TraceRef::Function(handle) => {
                    self.validate(handle)?;
                    (handle.object_id, 1)
                }
                TraceRef::Upvalue(handle) => {
                    self.validate(handle)?;
                    (handle.object_id, 2)
                }
            };
            let record = self
                .objects
                .get_mut(&object_id)
                .expect("validated object disappeared during mark");
            if record.marked {
                continue;
            }
            let actual_kind = match &record.object {
                HeapObject::Table(_) => 0,
                HeapObject::Closure(_) => 1,
                HeapObject::Upvalue(_) => 2,
            };
            if actual_kind != expected_kind {
                return Err(HeapError::WrongObjectKind { object_id });
            }
            record.marked = true;
            match &record.object {
                HeapObject::Table(table) => {
                    values.extend(table.array.iter().cloned());
                    for (key, value) in &table.map {
                        values.push(key.to_value());
                        values.push(value.clone());
                    }
                    if let Some(metatable) = table.metatable {
                        pending.push(TraceRef::Table(metatable));
                    }
                }
                HeapObject::Closure(closure) => {
                    Self::trace_proto(&closure.proto, &mut values);
                    pending.extend(closure.upvalues.iter().copied().map(TraceRef::Upvalue));
                }
                HeapObject::Upvalue(UpvalueState::Open(slot)) => values.push(
                    slot.try_borrow()
                        .map_err(|_| HeapError::StackBorrowConflict)?
                        .clone(),
                ),
                HeapObject::Upvalue(UpvalueState::Closed(value)) => values.push(value.clone()),
            }
        }
        Ok(())
    }

    fn trace_proto(proto: &Proto, values: &mut Vec<Value>) {
        values.extend(proto.constants.iter().cloned());
        for nested in &proto.protos {
            Self::trace_proto(nested, values);
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}
// ANCHOR_END: heap_owner
// ANCHOR_END: function_ref

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NativeFunction {
    Next,
    Pairs,
    IPairs,
    IPairsAux,
    SetMetatable,
    GetMetatable,
    RawGet,
    RawSet,
    ToString,
    SetTypeMetatable,
    GcCollect,
    GcLive,
}

impl NativeFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Pairs => "pairs",
            Self::IPairs => "ipairs",
            Self::IPairsAux => "ipairsaux",
            Self::SetMetatable => "setmetatable",
            Self::GetMetatable => "getmetatable",
            Self::RawGet => "rawget",
            Self::RawSet => "rawset",
            Self::ToString => "tostring",
            Self::SetTypeMetatable => "settypemetatable",
            Self::GcCollect => "gc_collect",
            Self::GcLive => "gc_live",
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
        ByteString, Heap, HeapError, Instruction, Opcode, StackSlot, TableError, Value, MAX_SBX,
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
        let mut heap = Heap::new();
        let slot: StackSlot = Rc::new(RefCell::new(Value::Number(1.0)));
        let upvalue = heap.allocate_open_upvalue(slot.clone()).unwrap();
        heap.upvalue_set(upvalue, Value::Number(2.0)).unwrap();
        assert_eq!(*slot.borrow(), Value::Number(2.0));
        heap.close_upvalue(upvalue).unwrap();
        assert!(!heap.upvalue_is_open(upvalue).unwrap());
        *slot.borrow_mut() = Value::Number(99.0);
        assert_eq!(heap.upvalue_get(upvalue), Ok(Value::Number(2.0)));
    }

    #[test]
    fn table_array_hash_and_deletion_rules_are_explicit() {
        let mut heap = Heap::new();
        let table = heap.allocate_table().unwrap();
        heap.table_set(table, Value::Number(1.0), Value::Number(10.0))
            .unwrap();
        heap.table_set(table, Value::Number(3.0), Value::Number(30.0))
            .unwrap();
        heap.table_set(
            table,
            Value::String(ByteString::new(&b"name"[..])),
            Value::String(ByteString::new(&b"lua"[..])),
        )
        .unwrap();
        assert_eq!(heap.table_array_len(table), Ok(1));
        assert_eq!(
            heap.table_get(table, &Value::Number(3.0)),
            Ok(Value::Number(30.0))
        );
        heap.table_set(table, Value::Number(2.0), Value::Number(20.0))
            .unwrap();
        assert_eq!(heap.table_array_len(table), Ok(3));
        heap.table_set(table, Value::Number(3.0), Value::Nil)
            .unwrap();
        assert_eq!(heap.table_array_len(table), Ok(2));
        assert_eq!(heap.table_get(table, &Value::Number(3.0)), Ok(Value::Nil));
    }

    #[test]
    fn invalid_keys_are_rejected() {
        let mut heap = Heap::new();
        let table = heap.allocate_table().unwrap();
        assert_eq!(
            heap.table_set(table, Value::Nil, Value::Number(1.0)),
            Err(HeapError::Table(TableError::NilKey))
        );
        assert_eq!(
            heap.table_set(table, Value::Number(f64::NAN), Value::Number(1.0)),
            Err(HeapError::Table(TableError::NanKey))
        );
    }

    #[test]
    fn table_metatable_can_be_attached_replaced_and_removed() {
        let mut heap = Heap::new();
        let table = heap.allocate_table().unwrap();
        let first = heap.allocate_table().unwrap();
        let second = heap.allocate_table().unwrap();

        assert_eq!(heap.table_metatable(table), Ok(None));
        heap.table_set_metatable(table, Some(first)).unwrap();
        assert_eq!(heap.table_metatable(table), Ok(Some(first)));
        heap.table_set_metatable(table, Some(second)).unwrap();
        assert_eq!(heap.table_metatable(table), Ok(Some(second)));
        heap.table_set_metatable(table, None).unwrap();
        assert_eq!(heap.table_metatable(table), Ok(None));
    }

    #[test]
    fn handles_share_identity_without_owning_the_object() {
        let mut heap = Heap::new();
        let table = heap.allocate_table().unwrap();
        let alias = table;
        heap.table_set(
            alias,
            Value::String(ByteString::new(&b"answer"[..])),
            Value::Number(42.0),
        )
        .unwrap();
        assert_eq!(
            heap.table_get(table, &Value::String(ByteString::new(&b"answer"[..]))),
            Ok(Value::Number(42.0))
        );
        assert_eq!(table, alias);
    }

    #[test]
    fn mark_and_sweep_collects_cycles_but_keeps_reachable_graphs() {
        let mut heap = Heap::new();
        let root = heap.allocate_table().unwrap();
        let child = heap.allocate_table().unwrap();
        heap.table_set(
            root,
            Value::String(ByteString::new(&b"child"[..])),
            Value::Table(child),
        )
        .unwrap();
        heap.table_set(
            child,
            Value::String(ByteString::new(&b"self"[..])),
            Value::Table(child),
        )
        .unwrap();

        let kept = heap.collect(&[Value::Table(root)], &[]).unwrap();
        assert_eq!((kept.collected, kept.remaining), (0, 2));
        assert_eq!(heap.is_live(child), Ok(true));

        let swept = heap.collect(&[], &[]).unwrap();
        assert_eq!((swept.collected, swept.remaining), (2, 0));
        assert_eq!(heap.is_live(child), Ok(false));
    }

    #[test]
    fn foreign_and_stale_handles_are_rejected() {
        let mut owner = Heap::new();
        let table = owner.allocate_table().unwrap();
        let mut other = Heap::new();
        assert!(matches!(
            other.table_get(table, &Value::Number(1.0)),
            Err(HeapError::ForeignHandle { .. })
        ));
        let other_table = other.allocate_table().unwrap();
        assert!(matches!(
            other.table_set(
                other_table,
                Value::String(ByteString::new(&b"foreign"[..])),
                Value::Table(table),
            ),
            Err(HeapError::ForeignHandle { .. })
        ));

        owner.collect(&[], &[]).unwrap();
        assert_eq!(
            owner.table_get(table, &Value::Number(1.0)),
            Err(HeapError::StaleHandle {
                object_id: table.object_id()
            })
        );
    }

    #[test]
    fn shutdown_releases_every_object_and_rejects_new_work() {
        let mut heap = Heap::new();
        heap.allocate_table().unwrap();
        heap.allocate_table().unwrap();
        assert_eq!(heap.shutdown(), 2);
        assert_eq!(heap.live_count(), 0);
        assert_eq!(heap.allocate_table(), Err(HeapError::Shutdown));
    }
}

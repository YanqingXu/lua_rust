use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
pub enum UserdataObject {}
pub enum CoroutineObject {}

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
pub type UserdataRef = GcRef<UserdataObject>;
pub type CoroutineRef = GcRef<CoroutineObject>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TableKey {
    Boolean(bool),
    Number(u64),
    String(ByteString),
    Table(TableRef),
    Function(FunctionRef),
    Userdata(UserdataRef),
    Coroutine(CoroutineRef),
    CoroutineWrapper(CoroutineRef),
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
            Value::Userdata(value) => Ok(Self::Userdata(*value)),
            Value::Coroutine(value) => Ok(Self::Coroutine(*value)),
            Value::CoroutineWrapper(value) => Ok(Self::CoroutineWrapper(*value)),
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
            Self::Userdata(value) => Value::Userdata(*value),
            Self::Coroutine(value) => Value::Coroutine(*value),
            Self::CoroutineWrapper(value) => Value::CoroutineWrapper(*value),
            Self::NativeFunction(value) => Value::NativeFunction(*value),
        }
    }

    fn object_id(&self) -> Option<u64> {
        match self {
            Self::Table(value) => Some(value.object_id),
            Self::Function(value) => Some(value.object_id),
            Self::Userdata(value) => Some(value.object_id),
            Self::Coroutine(value) | Self::CoroutineWrapper(value) => Some(value.object_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Table {
    array: Vec<Value>,
    map: HashMap<TableKey, Value>,
    metatable: Option<TableRef>,
    weak_keys: bool,
    weak_values: bool,
}

impl Table {
    fn new() -> Self {
        Self {
            array: Vec::new(),
            map: HashMap::new(),
            metatable: None,
            weak_keys: false,
            weak_values: false,
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
struct Userdata {
    finalizer: Option<Value>,
    finalized: bool,
}

#[derive(Clone, Debug)]
struct Coroutine {
    roots: Vec<Value>,
}

#[derive(Clone, Debug)]
enum HeapObject {
    Table(Table),
    Closure(Closure),
    Upvalue(UpvalueState),
    Userdata(Userdata),
    Coroutine(Coroutine),
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
    phase: GcPhase,
    gray: Vec<TraceRef>,
    sweep_ids: Vec<u64>,
    sweep_cursor: usize,
    cycle_before: usize,
    cycle_collected: usize,
    allocations_since_gc: usize,
    allocation_threshold: usize,
    ready_finalizers: Vec<FinalizerEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GcPhase {
    Pause,
    Mark,
    Atomic,
    Sweep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GcStep {
    pub phase: GcPhase,
    pub work: usize,
    pub completed: Option<GcStats>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinalizerEvent {
    pub userdata: UserdataRef,
    pub callback: Value,
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
    Userdata(UserdataRef),
    Coroutine(CoroutineRef),
}

impl Heap {
    pub fn new() -> Self {
        Self {
            heap_id: NEXT_HEAP_ID.fetch_add(1, AtomicOrdering::Relaxed),
            next_object_id: 1,
            objects: HashMap::new(),
            shutdown: false,
            phase: GcPhase::Pause,
            gray: Vec::new(),
            sweep_ids: Vec::new(),
            sweep_cursor: 0,
            cycle_before: 0,
            cycle_collected: 0,
            allocations_since_gc: 0,
            allocation_threshold: 64,
            ready_finalizers: Vec::new(),
        }
    }

    pub fn heap_id(&self) -> u64 {
        self.heap_id
    }

    pub fn live_count(&self) -> usize {
        self.objects.len()
    }

    pub fn phase(&self) -> GcPhase {
        self.phase
    }

    pub fn set_allocation_threshold(&mut self, threshold: usize) {
        self.allocation_threshold = threshold.max(1);
    }

    pub fn checkpoint_due(&self) -> bool {
        self.phase != GcPhase::Pause || self.allocations_since_gc >= self.allocation_threshold
    }

    pub fn take_finalizers(&mut self) -> Vec<FinalizerEvent> {
        std::mem::take(&mut self.ready_finalizers)
    }

    pub fn allocate_table(&mut self) -> Result<TableRef, HeapError> {
        let object_id = self.allocate(HeapObject::Table(Table::new()))?;
        let table = TableRef::new(self.heap_id, object_id);
        self.barrier_new(TraceRef::Table(table));
        Ok(table)
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
        let function = FunctionRef::new(self.heap_id, object_id);
        self.barrier_new(TraceRef::Function(function));
        Ok(function)
    }

    pub fn allocate_open_upvalue(&mut self, slot: StackSlot) -> Result<UpvalueRef, HeapError> {
        let object_id = self.allocate(HeapObject::Upvalue(UpvalueState::Open(slot)))?;
        let upvalue = UpvalueRef::new(self.heap_id, object_id);
        self.barrier_new(TraceRef::Upvalue(upvalue));
        Ok(upvalue)
    }

    pub fn allocate_userdata(&mut self) -> Result<UserdataRef, HeapError> {
        let object_id = self.allocate(HeapObject::Userdata(Userdata {
            finalizer: None,
            finalized: false,
        }))?;
        let userdata = UserdataRef::new(self.heap_id, object_id);
        self.barrier_new(TraceRef::Userdata(userdata));
        Ok(userdata)
    }

    pub fn allocate_coroutine(&mut self, roots: Vec<Value>) -> Result<CoroutineRef, HeapError> {
        for root in &roots {
            self.validate_value(root)?;
        }
        let object_id = self.allocate(HeapObject::Coroutine(Coroutine { roots }))?;
        let coroutine = CoroutineRef::new(self.heap_id, object_id);
        self.barrier_new(TraceRef::Coroutine(coroutine));
        Ok(coroutine)
    }

    pub fn coroutine_set_roots(
        &mut self,
        coroutine: CoroutineRef,
        roots: Vec<Value>,
    ) -> Result<(), HeapError> {
        for root in &roots {
            self.validate_value(root)?;
            self.write_barrier(coroutine.object_id, root)?;
        }
        self.coroutine_mut(coroutine)?.roots = roots;
        Ok(())
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
        self.write_barrier(table.object_id, &key)?;
        self.write_barrier(table.object_id, &value)?;
        let updates_mode = matches!(
            &key,
            Value::String(name) if name.as_bytes() == b"__mode"
        );
        let weak_mode = updates_mode.then(|| Self::parse_weak_mode(&value));
        self.table_mut(table)?
            .set(key, value)
            .map_err(HeapError::from)?;
        if let Some((weak_keys, weak_values)) = weak_mode {
            self.refresh_weak_dependents(table, weak_keys, weak_values);
        }
        Ok(())
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
            self.write_barrier(table.object_id, &Value::Table(metatable))?;
        }
        let (weak_keys, weak_values) = self.weak_mode_from_metatable(metatable)?;
        let target = self.table_mut(table)?;
        target.metatable = metatable;
        target.weak_keys = weak_keys;
        target.weak_values = weak_values;
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
        self.write_barrier(upvalue.object_id, &value)?;
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

    pub fn userdata_set_finalizer(
        &mut self,
        userdata: UserdataRef,
        finalizer: Option<Value>,
    ) -> Result<(), HeapError> {
        if let Some(finalizer) = &finalizer {
            self.validate_value(finalizer)?;
            self.write_barrier(userdata.object_id, finalizer)?;
        }
        let userdata = self.userdata_mut(userdata)?;
        userdata.finalizer = finalizer;
        userdata.finalized = false;
        Ok(())
    }

    pub fn userdata_is_finalized(&self, userdata: UserdataRef) -> Result<bool, HeapError> {
        Ok(self.userdata(userdata)?.finalized)
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
        loop {
            let step = self.step(roots, upvalue_roots, usize::MAX)?;
            if let Some(stats) = step.completed {
                return Ok(stats);
            }
        }
    }

    // ANCHOR: incremental_step
    pub fn step(
        &mut self,
        roots: &[Value],
        upvalue_roots: &[UpvalueRef],
        budget: usize,
    ) -> Result<GcStep, HeapError> {
        self.ensure_running()?;
        let budget = budget.max(1);
        if self.phase == GcPhase::Pause {
            self.start_cycle();
        }
        if matches!(self.phase, GcPhase::Mark | GcPhase::Atomic) {
            self.seed_roots(roots, upvalue_roots)?;
        }

        let mut work = 0;
        let mut completed = None;
        match self.phase {
            GcPhase::Pause => unreachable!("a step always starts a cycle"),
            GcPhase::Mark => {
                while work < budget && !self.gray.is_empty() {
                    self.mark_one()?;
                    work += 1;
                }
                if self.gray.is_empty() {
                    self.phase = GcPhase::Atomic;
                }
            }
            GcPhase::Atomic => {
                self.atomic_phase()?;
                work = 1;
            }
            GcPhase::Sweep => {
                while work < budget && self.sweep_cursor < self.sweep_ids.len() {
                    let object_id = self.sweep_ids[self.sweep_cursor];
                    self.sweep_cursor += 1;
                    let remove = self
                        .objects
                        .get(&object_id)
                        .is_some_and(|record| !record.marked);
                    if remove {
                        self.objects.remove(&object_id);
                        self.cycle_collected += 1;
                    }
                    work += 1;
                }
                if self.sweep_cursor == self.sweep_ids.len() {
                    for record in self.objects.values_mut() {
                        record.marked = false;
                    }
                    self.phase = GcPhase::Pause;
                    self.allocations_since_gc = 0;
                    completed = Some(GcStats {
                        before: self.cycle_before,
                        collected: self.cycle_collected,
                        remaining: self.objects.len(),
                    });
                }
            }
        }
        Ok(GcStep {
            phase: self.phase,
            work,
            completed,
        })
    }
    // ANCHOR_END: incremental_step
    // ANCHOR_END: mark_sweep

    pub fn shutdown(&mut self) -> usize {
        let released = self.objects.len();
        self.objects.clear();
        self.gray.clear();
        self.sweep_ids.clear();
        self.ready_finalizers.clear();
        self.phase = GcPhase::Pause;
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
                marked: self.phase == GcPhase::Sweep,
                object,
            },
        );
        self.allocations_since_gc = self.allocations_since_gc.saturating_add(1);
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
            Value::Userdata(userdata) => {
                self.userdata(*userdata)?;
            }
            Value::Coroutine(coroutine) | Value::CoroutineWrapper(coroutine) => {
                self.coroutine(*coroutine)?;
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

    fn userdata(&self, userdata: UserdataRef) -> Result<&Userdata, HeapError> {
        match &self.record(userdata)?.object {
            HeapObject::Userdata(userdata) => Ok(userdata),
            _ => Err(HeapError::WrongObjectKind {
                object_id: userdata.object_id,
            }),
        }
    }

    fn userdata_mut(&mut self, userdata: UserdataRef) -> Result<&mut Userdata, HeapError> {
        match &mut self.record_mut(userdata)?.object {
            HeapObject::Userdata(userdata) => Ok(userdata),
            _ => Err(HeapError::WrongObjectKind {
                object_id: userdata.object_id,
            }),
        }
    }

    fn coroutine(&self, coroutine: CoroutineRef) -> Result<&Coroutine, HeapError> {
        match &self.record(coroutine)?.object {
            HeapObject::Coroutine(coroutine) => Ok(coroutine),
            _ => Err(HeapError::WrongObjectKind {
                object_id: coroutine.object_id,
            }),
        }
    }

    fn coroutine_mut(&mut self, coroutine: CoroutineRef) -> Result<&mut Coroutine, HeapError> {
        match &mut self.record_mut(coroutine)?.object {
            HeapObject::Coroutine(coroutine) => Ok(coroutine),
            _ => Err(HeapError::WrongObjectKind {
                object_id: coroutine.object_id,
            }),
        }
    }

    fn start_cycle(&mut self) {
        for record in self.objects.values_mut() {
            record.marked = false;
        }
        self.gray.clear();
        self.sweep_ids.clear();
        self.sweep_cursor = 0;
        self.cycle_before = self.objects.len();
        self.cycle_collected = 0;
        self.phase = GcPhase::Mark;
    }

    fn seed_roots(
        &mut self,
        roots: &[Value],
        upvalue_roots: &[UpvalueRef],
    ) -> Result<(), HeapError> {
        for value in roots {
            self.enqueue_value(value)?;
        }
        for upvalue in upvalue_roots {
            self.validate(*upvalue)?;
            if !self.objects[&upvalue.object_id].marked {
                self.gray.push(TraceRef::Upvalue(*upvalue));
            }
        }
        Ok(())
    }

    fn enqueue_value(&mut self, value: &Value) -> Result<(), HeapError> {
        match value {
            Value::Table(table) => {
                self.table(*table)?;
                if !self.objects[&table.object_id].marked {
                    self.gray.push(TraceRef::Table(*table));
                }
            }
            Value::Function(function) => {
                self.closure(*function)?;
                if !self.objects[&function.object_id].marked {
                    self.gray.push(TraceRef::Function(*function));
                }
            }
            Value::Userdata(userdata) => {
                self.userdata(*userdata)?;
                if !self.objects[&userdata.object_id].marked {
                    self.gray.push(TraceRef::Userdata(*userdata));
                }
            }
            Value::Coroutine(coroutine) | Value::CoroutineWrapper(coroutine) => {
                self.coroutine(*coroutine)?;
                if !self.objects[&coroutine.object_id].marked {
                    self.gray.push(TraceRef::Coroutine(*coroutine));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn mark_one(&mut self) -> Result<(), HeapError> {
        let reference = self.gray.pop().expect("mark_one requires gray work");
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
            TraceRef::Userdata(handle) => {
                self.validate(handle)?;
                (handle.object_id, 3)
            }
            TraceRef::Coroutine(handle) => {
                self.validate(handle)?;
                (handle.object_id, 4)
            }
        };
        let record = self
            .objects
            .get_mut(&object_id)
            .expect("validated object disappeared during mark");
        if record.marked {
            return Ok(());
        }
        let actual_kind = match &record.object {
            HeapObject::Table(_) => 0,
            HeapObject::Closure(_) => 1,
            HeapObject::Upvalue(_) => 2,
            HeapObject::Userdata(_) => 3,
            HeapObject::Coroutine(_) => 4,
        };
        if actual_kind != expected_kind {
            return Err(HeapError::WrongObjectKind { object_id });
        }
        record.marked = true;

        let mut values = Vec::new();
        let mut references = Vec::new();
        match &record.object {
            HeapObject::Table(table) => {
                if !table.weak_values {
                    values.extend(table.array.iter().cloned());
                }
                for (key, value) in &table.map {
                    if !table.weak_keys {
                        values.push(key.to_value());
                    }
                    if !table.weak_values && !table.weak_keys {
                        values.push(value.clone());
                    }
                }
                if let Some(metatable) = table.metatable {
                    references.push(TraceRef::Table(metatable));
                }
            }
            HeapObject::Closure(closure) => {
                Self::trace_proto(&closure.proto, &mut values);
                references.extend(closure.upvalues.iter().copied().map(TraceRef::Upvalue));
            }
            HeapObject::Upvalue(UpvalueState::Open(slot)) => values.push(
                slot.try_borrow()
                    .map_err(|_| HeapError::StackBorrowConflict)?
                    .clone(),
            ),
            HeapObject::Upvalue(UpvalueState::Closed(value)) => values.push(value.clone()),
            HeapObject::Userdata(userdata) => {
                if let Some(finalizer) = &userdata.finalizer {
                    values.push(finalizer.clone());
                }
            }
            HeapObject::Coroutine(coroutine) => values.extend(coroutine.roots.iter().cloned()),
        }
        self.gray.extend(references);
        for value in &values {
            self.enqueue_value(value)?;
        }
        Ok(())
    }

    // ANCHOR: atomic_weak_finalizer
    fn atomic_phase(&mut self) -> Result<(), HeapError> {
        loop {
            let mut ephemeron_values = Vec::new();
            for record in self.objects.values() {
                let HeapObject::Table(table) = &record.object else {
                    continue;
                };
                if !record.marked || !table.weak_keys || table.weak_values {
                    continue;
                }
                for (key, value) in &table.map {
                    if self.key_is_marked(key) && !self.value_is_marked(value) {
                        ephemeron_values.push(value.clone());
                    }
                }
            }
            if ephemeron_values.is_empty() {
                break;
            }
            for value in &ephemeron_values {
                self.enqueue_value(value)?;
            }
            while !self.gray.is_empty() {
                self.mark_one()?;
            }
        }

        let mut finalizable = self
            .objects
            .iter()
            .filter_map(|(&object_id, record)| match &record.object {
                HeapObject::Userdata(userdata)
                    if !record.marked && !userdata.finalized && userdata.finalizer.is_some() =>
                {
                    Some(object_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        finalizable.sort_unstable();
        for object_id in finalizable {
            let userdata_ref = UserdataRef::new(self.heap_id, object_id);
            let callback = match &mut self
                .objects
                .get_mut(&object_id)
                .expect("finalizable object exists")
                .object
            {
                HeapObject::Userdata(userdata) => {
                    userdata.finalized = true;
                    userdata
                        .finalizer
                        .clone()
                        .expect("filtered finalizer exists")
                }
                _ => unreachable!(),
            };
            self.ready_finalizers.push(FinalizerEvent {
                userdata: userdata_ref,
                callback: callback.clone(),
            });
            self.gray.push(TraceRef::Userdata(userdata_ref));
            self.enqueue_value(&callback)?;
        }
        while !self.gray.is_empty() {
            self.mark_one()?;
        }

        self.cleanup_weak_tables();
        self.sweep_ids = self.objects.keys().copied().collect();
        self.sweep_ids.sort_unstable();
        self.sweep_cursor = 0;
        self.phase = GcPhase::Sweep;
        Ok(())
    }
    // ANCHOR_END: atomic_weak_finalizer

    fn cleanup_weak_tables(&mut self) {
        let marked = self
            .objects
            .iter()
            .filter_map(|(&object_id, record)| record.marked.then_some(object_id))
            .collect::<HashSet<_>>();
        for record in self.objects.values_mut() {
            let HeapObject::Table(table) = &mut record.object else {
                continue;
            };
            if table.weak_values {
                for value in &mut table.array {
                    if !Self::value_is_alive_in(&marked, value) {
                        *value = Value::Nil;
                    }
                }
                table.trim_array();
            }
            table.map.retain(|key, value| {
                (!table.weak_keys || Self::key_is_alive_in(&marked, key))
                    && (!table.weak_values || Self::value_is_alive_in(&marked, value))
            });
        }
    }

    fn key_is_marked(&self, key: &TableKey) -> bool {
        match key.object_id() {
            Some(object_id) => self
                .objects
                .get(&object_id)
                .is_some_and(|record| record.marked),
            None => true,
        }
    }

    fn value_is_marked(&self, value: &Value) -> bool {
        match Self::value_object_id(value) {
            Some(object_id) => self
                .objects
                .get(&object_id)
                .is_some_and(|record| record.marked),
            None => true,
        }
    }

    fn key_is_alive_in(marked: &HashSet<u64>, key: &TableKey) -> bool {
        key.object_id()
            .is_none_or(|object_id| marked.contains(&object_id))
    }

    fn value_is_alive_in(marked: &HashSet<u64>, value: &Value) -> bool {
        Self::value_object_id(value).is_none_or(|object_id| marked.contains(&object_id))
    }

    fn value_object_id(value: &Value) -> Option<u64> {
        match value {
            Value::Table(value) => Some(value.object_id),
            Value::Function(value) => Some(value.object_id),
            Value::Userdata(value) => Some(value.object_id),
            Value::Coroutine(value) | Value::CoroutineWrapper(value) => Some(value.object_id),
            _ => None,
        }
    }

    // ANCHOR: write_barrier
    fn barrier_new(&mut self, reference: TraceRef) {
        match self.phase {
            GcPhase::Mark | GcPhase::Atomic => self.gray.push(reference),
            GcPhase::Sweep => {
                let object_id = match reference {
                    TraceRef::Table(value) => value.object_id,
                    TraceRef::Function(value) => value.object_id,
                    TraceRef::Upvalue(value) => value.object_id,
                    TraceRef::Userdata(value) => value.object_id,
                    TraceRef::Coroutine(value) => value.object_id,
                };
                if let Some(record) = self.objects.get_mut(&object_id) {
                    record.marked = true;
                }
            }
            GcPhase::Pause => {}
        }
    }

    fn write_barrier(&mut self, owner_id: u64, value: &Value) -> Result<(), HeapError> {
        let owner_marked = self
            .objects
            .get(&owner_id)
            .ok_or(HeapError::StaleHandle {
                object_id: owner_id,
            })?
            .marked;
        if owner_marked && matches!(self.phase, GcPhase::Mark | GcPhase::Atomic) {
            self.enqueue_value(value)?;
        }
        Ok(())
    }
    // ANCHOR_END: write_barrier

    fn weak_mode_from_metatable(
        &self,
        metatable: Option<TableRef>,
    ) -> Result<(bool, bool), HeapError> {
        let Some(metatable) = metatable else {
            return Ok((false, false));
        };
        let mode = self
            .table(metatable)?
            .get(&Value::String(ByteString::new(&b"__mode"[..])))?;
        Ok(Self::parse_weak_mode(&mode))
    }

    fn parse_weak_mode(value: &Value) -> (bool, bool) {
        let Value::String(mode) = value else {
            return (false, false);
        };
        (
            mode.as_bytes().contains(&b'k'),
            mode.as_bytes().contains(&b'v'),
        )
    }

    fn refresh_weak_dependents(&mut self, metatable: TableRef, weak_keys: bool, weak_values: bool) {
        for record in self.objects.values_mut() {
            if let HeapObject::Table(table) = &mut record.object {
                if table.metatable == Some(metatable) {
                    table.weak_keys = weak_keys;
                    table.weak_values = weak_values;
                }
            }
        }
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

// ANCHOR: stdlib_native_ids
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BaseFunction {
    Assert,
    Error,
    Type,
    ToNumber,
    RawEqual,
    Select,
    Unpack,
    PCall,
    XPCall,
    Print,
    CollectGarbage,
}

impl BaseFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assert => "assert",
            Self::Error => "error",
            Self::Type => "type",
            Self::ToNumber => "tonumber",
            Self::RawEqual => "rawequal",
            Self::Select => "select",
            Self::Unpack => "unpack",
            Self::PCall => "pcall",
            Self::XPCall => "xpcall",
            Self::Print => "print",
            Self::CollectGarbage => "collectgarbage",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MathFunction {
    Abs,
    Acos,
    Asin,
    Atan,
    Atan2,
    Ceil,
    Cos,
    Cosh,
    Deg,
    Exp,
    Floor,
    Fmod,
    Frexp,
    Ldexp,
    Log,
    Log10,
    Max,
    Min,
    Modf,
    Pow,
    Rad,
    Random,
    RandomSeed,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
}

impl MathFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Abs => "math.abs",
            Self::Acos => "math.acos",
            Self::Asin => "math.asin",
            Self::Atan => "math.atan",
            Self::Atan2 => "math.atan2",
            Self::Ceil => "math.ceil",
            Self::Cos => "math.cos",
            Self::Cosh => "math.cosh",
            Self::Deg => "math.deg",
            Self::Exp => "math.exp",
            Self::Floor => "math.floor",
            Self::Fmod => "math.fmod",
            Self::Frexp => "math.frexp",
            Self::Ldexp => "math.ldexp",
            Self::Log => "math.log",
            Self::Log10 => "math.log10",
            Self::Max => "math.max",
            Self::Min => "math.min",
            Self::Modf => "math.modf",
            Self::Pow => "math.pow",
            Self::Rad => "math.rad",
            Self::Random => "math.random",
            Self::RandomSeed => "math.randomseed",
            Self::Sin => "math.sin",
            Self::Sinh => "math.sinh",
            Self::Sqrt => "math.sqrt",
            Self::Tan => "math.tan",
            Self::Tanh => "math.tanh",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StringFunction {
    Byte,
    Char,
    Find,
    Format,
    Len,
    Lower,
    Rep,
    Reverse,
    Sub,
    Upper,
}

impl StringFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Byte => "string.byte",
            Self::Char => "string.char",
            Self::Find => "string.find",
            Self::Format => "string.format",
            Self::Len => "string.len",
            Self::Lower => "string.lower",
            Self::Rep => "string.rep",
            Self::Reverse => "string.reverse",
            Self::Sub => "string.sub",
            Self::Upper => "string.upper",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TableFunction {
    Concat,
    Insert,
    MaxN,
    Remove,
    Sort,
}

impl TableFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Concat => "table.concat",
            Self::Insert => "table.insert",
            Self::MaxN => "table.maxn",
            Self::Remove => "table.remove",
            Self::Sort => "table.sort",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NativeFunction {
    Base(BaseFunction),
    Math(MathFunction),
    String(StringFunction),
    Table(TableFunction),
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
    GcStep,
    GcPhase,
    NewUserdata,
    SetFinalizer,
    UserdataId,
    CoroutineCreate,
    CoroutineResume,
    CoroutineYield,
    CoroutineStatus,
    CoroutineRunning,
    CoroutineWrap,
}

impl NativeFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Base(function) => function.name(),
            Self::Math(function) => function.name(),
            Self::String(function) => function.name(),
            Self::Table(function) => function.name(),
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
            Self::GcStep => "gc_step",
            Self::GcPhase => "gc_phase",
            Self::NewUserdata => "newuserdata",
            Self::SetFinalizer => "setfinalizer",
            Self::UserdataId => "userdataid",
            Self::CoroutineCreate => "coroutine.create",
            Self::CoroutineResume => "coroutine.resume",
            Self::CoroutineYield => "coroutine.yield",
            Self::CoroutineStatus => "coroutine.status",
            Self::CoroutineRunning => "coroutine.running",
            Self::CoroutineWrap => "coroutine.wrap",
        }
    }
}
// ANCHOR_END: stdlib_native_ids

#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(ByteString),
    Table(TableRef),
    Function(FunctionRef),
    Userdata(UserdataRef),
    Coroutine(CoroutineRef),
    CoroutineWrapper(CoroutineRef),
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
            (Self::Userdata(left), Self::Userdata(right)) => left == right,
            (Self::Coroutine(left), Self::Coroutine(right)) => left == right,
            (Self::CoroutineWrapper(left), Self::CoroutineWrapper(right)) => left == right,
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
            Self::Userdata(_) => "userdata",
            Self::Coroutine(_) => "thread",
            Self::CoroutineWrapper(_) => "function",
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
            Self::Userdata(value) => write!(formatter, "userdata: 0x{:x}", value.identity()),
            Self::Coroutine(value) => write!(formatter, "thread: 0x{:x}", value.identity()),
            Self::CoroutineWrapper(value) => {
                write!(formatter, "function: coroutine:0x{:x}", value.identity())
            }
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

    #[test]
    fn incremental_and_full_collection_reach_the_same_result() {
        fn graph() -> (Heap, Value) {
            let mut heap = Heap::new();
            let root = heap.allocate_table().unwrap();
            let child = heap.allocate_table().unwrap();
            let garbage = heap.allocate_table().unwrap();
            heap.table_set(
                root,
                Value::String(ByteString::new(&b"child"[..])),
                Value::Table(child),
            )
            .unwrap();
            heap.table_set(
                garbage,
                Value::String(ByteString::new(&b"self"[..])),
                Value::Table(garbage),
            )
            .unwrap();
            (heap, Value::Table(root))
        }

        let (mut full, full_root) = graph();
        let full_stats = full.collect(&[full_root], &[]).unwrap();

        let (mut incremental, incremental_root) = graph();
        let mut phases = Vec::new();
        let incremental_stats = loop {
            let step = incremental
                .step(std::slice::from_ref(&incremental_root), &[], 1)
                .unwrap();
            phases.push(step.phase);
            if let Some(stats) = step.completed {
                break stats;
            }
        };
        assert_eq!(incremental_stats, full_stats);
        assert!(phases.contains(&super::GcPhase::Atomic));
        assert!(phases.contains(&super::GcPhase::Sweep));
    }

    #[test]
    fn write_barrier_keeps_a_white_child_added_to_a_marked_table() {
        let mut heap = Heap::new();
        let parent = heap.allocate_table().unwrap();
        let child = heap.allocate_table().unwrap();
        let root = Value::Table(parent);

        let step = heap.step(std::slice::from_ref(&root), &[], 1).unwrap();
        assert_eq!(step.phase, super::GcPhase::Atomic);
        heap.table_set(
            parent,
            Value::String(ByteString::new(&b"late"[..])),
            Value::Table(child),
        )
        .unwrap();
        let stats = heap.collect(&[root], &[]).unwrap();
        assert_eq!(stats.collected, 0);
        assert_eq!(heap.is_live(child), Ok(true));
    }

    #[test]
    fn weak_values_are_cleared_and_weak_keys_use_ephemeron_rules() {
        let mut heap = Heap::new();
        let mode = heap.allocate_table().unwrap();
        heap.table_set(
            mode,
            Value::String(ByteString::new(&b"__mode"[..])),
            Value::String(ByteString::new(&b"kv"[..])),
        )
        .unwrap();
        let weak = heap.allocate_table().unwrap();
        heap.table_set_metatable(weak, Some(mode)).unwrap();
        let key = heap.allocate_table().unwrap();
        let value = heap.allocate_table().unwrap();
        heap.table_set(weak, Value::Table(key), Value::Table(value))
            .unwrap();

        heap.collect(&[Value::Table(weak)], &[]).unwrap();
        assert_eq!(heap.table_next_pair(weak, &Value::Nil), Ok(None));
        assert_eq!(heap.is_live(key), Ok(false));
        assert_eq!(heap.is_live(value), Ok(false));
    }

    #[test]
    fn finalizable_userdata_survives_one_cycle_then_has_a_second_death() {
        let mut heap = Heap::new();
        let userdata = heap.allocate_userdata().unwrap();
        heap.userdata_set_finalizer(
            userdata,
            Some(Value::NativeFunction(super::NativeFunction::ToString)),
        )
        .unwrap();

        let first = heap.collect(&[], &[]).unwrap();
        assert_eq!(first.collected, 0);
        let events = heap.take_finalizers();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].userdata, userdata);
        assert_eq!(heap.userdata_is_finalized(userdata), Ok(true));

        let second = heap.collect(&[], &[]).unwrap();
        assert_eq!(second.collected, 1);
        assert!(heap.take_finalizers().is_empty());
        assert_eq!(heap.is_live(userdata), Ok(false));
    }

    #[test]
    fn allocator_accounting_requests_an_automatic_checkpoint() {
        let mut heap = Heap::new();
        heap.set_allocation_threshold(2);
        assert!(!heap.checkpoint_due());
        heap.allocate_table().unwrap();
        assert!(!heap.checkpoint_due());
        heap.allocate_table().unwrap();
        assert!(heap.checkpoint_due());
    }
}

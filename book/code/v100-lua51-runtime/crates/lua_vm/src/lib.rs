use lua_compiler::compile;
use lua_core::{
    BaseFunction, ByteString, CoroutineRef, DebugFunction, FileFunction, FunctionRef,
    GcPhase as HeapPhase, GcStats, GcStep, Heap, HeapError, IoFunction, MathFunction,
    NativeFunction, Opcode, OsFunction, PackageFunction, Proto, StackSlot, StringFunction,
    TableFunction, TableRef, UpvalueRef, UserdataRef, Value,
};
use lua_stdlib::{Args, HostError, BASE_FUNCTIONS, FILE_FUNCTIONS, LIBRARIES};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const METAMETHOD_CHAIN_LIMIT: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmError {
    pub kind: VmErrorKind,
    pub pc: usize,
    pub line: usize,
    pub message: String,
    pub traceback: Vec<String>,
}

// ANCHOR: coroutine_state
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmErrorKind {
    Bytecode,
    Type,
    Metamethod,
    Limit,
    Runtime,
    Yield,
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
    remaining_instructions: usize,
    global_env: TableRef,
    max_call_depth: usize,
    call_stack: Vec<CallInfo>,
    next_frame_id: u64,
    open_upvalues: HashMap<(u64, u8), UpvalueRef>,
    type_metatables: HashMap<&'static str, TableRef>,
    heap: Heap,
    registry: TableRef,
    active_frames: HashMap<u64, Vec<StackSlot>>,
    active_functions: Vec<FunctionRef>,
    temporary_roots: Vec<Value>,
    manual_gc_mode: bool,
    coroutines: HashMap<CoroutineRef, CoroutineState>,
    current_coroutine: Option<CoroutineRef>,
    yield_values: Option<Vec<Value>>,
    yield_frames: Vec<SuspendedFrame>,
    yield_forbidden_depth: usize,
    random_state: u64,
    files: HashMap<UserdataRef, Option<File>>,
    file_metatable: TableRef,
    package_loaded: TableRef,
    package_preload: TableRef,
    start_time: Instant,
    temporary_name_counter: u64,
    trace_enabled: bool,
    trace_events: Vec<VmTraceEvent>,
}

#[derive(Clone, Debug)]
struct CallInfo {
    source: String,
    line: usize,
    base: usize,
    top: usize,
}

enum FrameOutcome {
    Return(Vec<Value>),
    TailCall(FunctionRef, Vec<Value>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoroutineStatus {
    Suspended,
    Running,
    Normal,
    Dead,
}

impl CoroutineStatus {
    fn name(self) -> &'static [u8] {
        match self {
            Self::Suspended => b"suspended",
            Self::Running => b"running",
            Self::Normal => b"normal",
            Self::Dead => b"dead",
        }
    }
}

struct CoroutineState {
    status: CoroutineStatus,
    entry: Option<Value>,
    frames: Vec<SuspendedFrame>,
}

struct SuspendedFrame {
    function: FunctionRef,
    registers: Vec<StackSlot>,
    varargs: Vec<Value>,
    pc: usize,
    frame_id: u64,
    resume_mode: ResumeMode,
}

enum ResumeMode {
    Call { base: usize, wanted: Option<usize> },
    Tail,
}
// ANCHOR_END: coroutine_state

#[derive(Clone, Copy)]
struct DateParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    weekday: i64,
    yearday: i64,
}

#[derive(Clone, Debug)]
pub struct VmTraceEvent {
    pub frame_depth: usize,
    pub function: String,
    pub pc: usize,
    pub line: usize,
    pub opcode: Opcode,
    pub registers: Vec<Value>,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let registry = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate its registry");
        let global_env = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate the global environment");
        for spec in BASE_FUNCTIONS {
            heap.table_set(
                global_env,
                Value::String(ByteString::new(spec.name)),
                Value::NativeFunction(spec.function),
            )
            .expect("a fresh teaching heap can install a base function");
        }
        for (name, function) in [
            (
                b"settypemetatable".as_slice(),
                NativeFunction::SetTypeMetatable,
            ),
            (b"gc_collect".as_slice(), NativeFunction::GcCollect),
            (b"gc_live".as_slice(), NativeFunction::GcLive),
            (b"gc_step".as_slice(), NativeFunction::GcStep),
            (b"gc_phase".as_slice(), NativeFunction::GcPhase),
            (b"newuserdata".as_slice(), NativeFunction::NewUserdata),
            (b"setfinalizer".as_slice(), NativeFunction::SetFinalizer),
            (b"userdataid".as_slice(), NativeFunction::UserdataId),
        ] {
            heap.table_set(
                global_env,
                Value::String(ByteString::new(name)),
                Value::NativeFunction(function),
            )
            .expect("a fresh teaching heap can install a diagnostic function");
        }
        let mut string_library = None;
        let mut package_library = None;
        for library in LIBRARIES {
            let table = heap
                .allocate_table()
                .expect("a fresh teaching heap can allocate a standard library table");
            for spec in library.functions {
                heap.table_set(
                    table,
                    Value::String(ByteString::new(spec.name)),
                    Value::NativeFunction(spec.function),
                )
                .expect("a fresh teaching heap can install a library function");
            }
            if library.name == b"math" {
                for (name, value) in [
                    (b"pi".as_slice(), std::f64::consts::PI),
                    (b"huge", f64::INFINITY),
                ] {
                    heap.table_set(
                        table,
                        Value::String(ByteString::new(name)),
                        Value::Number(value),
                    )
                    .expect("a fresh teaching heap can install a math constant");
                }
            }
            if library.name == b"string" {
                string_library = Some(table);
            }
            if library.name == b"package" {
                package_library = Some(table);
            }
            heap.table_set(
                global_env,
                Value::String(ByteString::new(library.name)),
                Value::Table(table),
            )
            .expect("a fresh teaching heap can publish a standard library");
        }
        heap.table_set(
            global_env,
            Value::String(ByteString::new(&b"_G"[..])),
            Value::Table(global_env),
        )
        .expect("a fresh teaching heap can install _G");
        heap.table_set(
            global_env,
            Value::String(ByteString::new(&b"_VERSION"[..])),
            Value::String(ByteString::new(&b"Lua 5.1"[..])),
        )
        .expect("a fresh teaching heap can install _VERSION");
        let mut type_metatables = HashMap::new();
        if let Some(string_library) = string_library {
            let string_metatable = heap
                .allocate_table()
                .expect("a fresh teaching heap can allocate the string metatable");
            heap.table_set(
                string_metatable,
                Value::String(ByteString::new(&b"__index"[..])),
                Value::Table(string_library),
            )
            .expect("a fresh teaching heap can install string methods");
            type_metatables.insert("string", string_metatable);
        }
        let package_library = package_library.expect("the catalog contains package");
        let package_loaded = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate package.loaded");
        let package_preload = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate package.preload");
        for (name, value) in [
            (b"loaded".as_slice(), Value::Table(package_loaded)),
            (b"preload".as_slice(), Value::Table(package_preload)),
            (
                b"path".as_slice(),
                Value::String(ByteString::new(&b"./?.lua;./?/init.lua"[..])),
            ),
        ] {
            heap.table_set(package_library, Value::String(ByteString::new(name)), value)
                .expect("a fresh teaching heap can initialize package state");
        }
        let file_metatable = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate the file metatable");
        for spec in FILE_FUNCTIONS {
            heap.table_set(
                file_metatable,
                Value::String(ByteString::new(spec.name)),
                Value::NativeFunction(spec.function),
            )
            .expect("a fresh teaching heap can install a file method");
        }
        heap.table_set(
            file_metatable,
            Value::String(ByteString::new(&b"__index"[..])),
            Value::Table(file_metatable),
        )
        .expect("a fresh teaching heap can make file methods indexable");
        Self {
            instruction_limit: 1_000_000,
            remaining_instructions: 1_000_000,
            global_env,
            max_call_depth: 32,
            call_stack: Vec::new(),
            next_frame_id: 1,
            open_upvalues: HashMap::new(),
            type_metatables,
            heap,
            registry,
            active_frames: HashMap::new(),
            active_functions: Vec::new(),
            temporary_roots: Vec::new(),
            manual_gc_mode: false,
            coroutines: HashMap::new(),
            current_coroutine: None,
            yield_values: None,
            yield_frames: Vec::new(),
            yield_forbidden_depth: 0,
            random_state: 0x4d59_5df4_d0f3_3173,
            files: HashMap::new(),
            file_metatable,
            package_loaded,
            package_preload,
            start_time: Instant::now(),
            temporary_name_counter: 0,
            trace_enabled: false,
            trace_events: Vec::new(),
        }
    }

    pub fn with_instruction_limit(instruction_limit: usize) -> Self {
        let mut vm = Self::new();
        vm.instruction_limit = instruction_limit;
        vm
    }

    pub fn with_call_depth_limit(mut self, max_call_depth: usize) -> Self {
        self.max_call_depth = max_call_depth;
        self
    }

    pub fn with_gc_allocation_threshold(mut self, threshold: usize) -> Self {
        self.heap.set_allocation_threshold(threshold);
        self
    }

    pub fn heap_live_count(&self) -> usize {
        self.heap.live_count()
    }

    pub fn coroutine_state_count(&self) -> usize {
        self.coroutines.len()
    }

    pub fn enable_trace(&mut self) {
        self.trace_events.clear();
        self.trace_enabled = true;
    }

    pub fn take_trace(&mut self) -> Vec<VmTraceEvent> {
        self.trace_enabled = false;
        std::mem::take(&mut self.trace_events)
    }

    pub fn set_arg_table(&mut self, entries: &[(i64, Vec<u8>)]) -> Result<(), HeapError> {
        let table = self.heap.allocate_table()?;
        for (index, bytes) in entries {
            self.heap.table_set(
                table,
                Value::Number(*index as f64),
                Value::String(ByteString::new(bytes.as_slice())),
            )?;
        }
        self.heap.table_set(
            self.global_env,
            Value::String(ByteString::new(&b"arg"[..])),
            Value::Table(table),
        )
    }

    pub fn set_global(&mut self, name: &[u8], value: Value) -> Result<(), HeapError> {
        self.heap
            .table_set(self.global_env, Value::String(ByteString::new(name)), value)
    }

    pub fn registry(&self) -> TableRef {
        self.registry
    }

    pub fn registry_get(&self, key: &Value) -> Result<Value, HeapError> {
        self.heap.table_get(self.registry, key)
    }

    pub fn registry_set(&mut self, key: Value, value: Value) -> Result<(), HeapError> {
        self.heap.table_set(self.registry, key, value)
    }

    // ANCHOR: gc_roots
    fn gc_roots(&self) -> Result<(Vec<Value>, Vec<UpvalueRef>), HeapError> {
        let mut roots = vec![Value::Table(self.global_env)];
        roots.push(Value::Table(self.registry));
        roots.push(Value::Table(self.file_metatable));
        roots.push(Value::Table(self.package_loaded));
        roots.push(Value::Table(self.package_preload));
        roots.extend(self.type_metatables.values().copied().map(Value::Table));
        roots.extend(self.active_functions.iter().copied().map(Value::Function));
        roots.extend(self.temporary_roots.iter().cloned());
        for slots in self.active_frames.values() {
            for slot in slots {
                roots.push(
                    slot.try_borrow()
                        .map_err(|_| HeapError::StackBorrowConflict)?
                        .clone(),
                );
            }
        }
        let upvalues = self
            .open_upvalues
            .iter()
            .filter(|((owner, _), _)| self.active_frames.contains_key(owner))
            .map(|(_, upvalue)| *upvalue)
            .collect::<Vec<_>>();
        Ok((roots, upvalues))
    }

    pub fn collect_garbage(&mut self) -> Result<GcStats, VmError> {
        let context = Proto {
            constants: Vec::new(),
            code: Vec::new(),
            lines: vec![0],
            max_stack_size: 1,
            protos: Vec::new(),
            num_params: 0,
            is_vararg: false,
            upvalues: Vec::new(),
            debug_name: "host-gc".to_owned(),
        };
        self.finish_gc(&context, 0)
    }

    fn finish_gc(&mut self, proto: &Proto, pc: usize) -> Result<GcStats, VmError> {
        loop {
            let step = self.gc_step_internal(usize::MAX, proto, pc)?;
            if let Some(stats) = step.completed {
                return Ok(stats);
            }
        }
    }

    fn gc_step_internal(
        &mut self,
        budget: usize,
        proto: &Proto,
        pc: usize,
    ) -> Result<GcStep, VmError> {
        let (roots, upvalues) = self
            .gc_roots()
            .map_err(|error| self.error(proto, pc, error.to_string()))?;
        let step = self
            .heap
            .step(&roots, &upvalues, budget)
            .map_err(|error| self.error(proto, pc, error.to_string()))?;
        let finalizers = self.heap.take_finalizers();
        let root_base = self.temporary_roots.len();
        for finalizer in &finalizers {
            self.temporary_roots.push(finalizer.callback.clone());
            self.temporary_roots
                .push(Value::Userdata(finalizer.userdata));
        }
        let mut first_error = None;
        for finalizer in finalizers {
            self.yield_forbidden_depth += 1;
            let result = self.invoke_callable(
                finalizer.callback,
                vec![Value::Userdata(finalizer.userdata)],
                proto,
                pc,
            );
            self.yield_forbidden_depth -= 1;
            if let Err(error) = result {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        if step.completed.is_some() {
            self.prune_collected_coroutines();
            self.files
                .retain(|userdata, _| matches!(self.heap.is_live(*userdata), Ok(true)));
        }
        self.temporary_roots.truncate(root_base);
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(step)
    }
    // ANCHOR_END: gc_roots

    // ANCHOR: dispatch
    pub fn execute(&mut self, proto: &Proto) -> Result<Value, VmError> {
        Ok(self
            .execute_multi(proto)?
            .into_iter()
            .next()
            .unwrap_or(Value::Nil))
    }

    pub fn execute_multi(&mut self, proto: &Proto) -> Result<Vec<Value>, VmError> {
        self.call_stack.clear();
        self.active_frames.clear();
        self.active_functions.clear();
        self.temporary_roots.clear();
        self.manual_gc_mode = false;
        self.remaining_instructions = self.instruction_limit;
        let function = self
            .heap
            .allocate_function(Rc::new(proto.clone()))
            .map_err(|error| VmError {
                kind: VmErrorKind::Runtime,
                pc: 0,
                line: 0,
                message: error.to_string(),
                traceback: Vec::new(),
            })?;
        self.execute_proto(function, &[])
    }

    // ANCHOR: call_frame
    fn execute_proto(
        &mut self,
        function: FunctionRef,
        arguments: &[Value],
    ) -> Result<Vec<Value>, VmError> {
        let mut function = function;
        let mut arguments = arguments.to_vec();
        loop {
            match self.execute_frame(&function, &arguments)? {
                FrameOutcome::Return(results) => return Ok(results),
                FrameOutcome::TailCall(next_function, next_arguments) => {
                    function = next_function;
                    arguments = next_arguments;
                }
            }
        }
    }

    fn execute_frame(
        &mut self,
        function: &FunctionRef,
        arguments: &[Value],
    ) -> Result<FrameOutcome, VmError> {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.wrapping_add(1);
        self.active_functions.push(*function);
        let result = self.run_frame(function, arguments, frame_id);
        if matches!(&result, Err(error) if error.kind == VmErrorKind::Yield) {
            self.active_frames.remove(&frame_id);
            self.active_functions.pop();
            return result;
        }
        let close_result = self.close_upvalues(frame_id, 0);
        self.active_frames.remove(&frame_id);
        self.active_functions.pop();
        match (result, close_result) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(VmError {
                kind: VmErrorKind::Runtime,
                pc: 0,
                line: 0,
                message: error.to_string(),
                traceback: Vec::new(),
            }),
        }
    }

    fn resume_frame(
        &mut self,
        frame: SuspendedFrame,
        resume_values: Vec<Value>,
    ) -> Result<FrameOutcome, VmError> {
        let function = frame.function;
        let frame_id = frame.frame_id;
        self.active_functions.push(function);
        let result = self.run_frame_state(&function, &[], frame_id, Some((frame, resume_values)));
        if matches!(&result, Err(error) if error.kind == VmErrorKind::Yield) {
            self.active_frames.remove(&frame_id);
            self.active_functions.pop();
            return result;
        }
        let close_result = self.close_upvalues(frame_id, 0);
        self.active_frames.remove(&frame_id);
        self.active_functions.pop();
        match (result, close_result) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(VmError {
                kind: VmErrorKind::Runtime,
                pc: 0,
                line: 0,
                message: error.to_string(),
                traceback: Vec::new(),
            }),
        }
    }

    fn run_frame(
        &mut self,
        function: &FunctionRef,
        arguments: &[Value],
        frame_id: u64,
    ) -> Result<FrameOutcome, VmError> {
        self.run_frame_state(function, arguments, frame_id, None)
    }

    fn run_frame_state(
        &mut self,
        function: &FunctionRef,
        arguments: &[Value],
        frame_id: u64,
        suspended: Option<(SuspendedFrame, Vec<Value>)>,
    ) -> Result<FrameOutcome, VmError> {
        let proto_owner = self
            .heap
            .function_proto(*function)
            .map_err(|error| VmError {
                kind: VmErrorKind::Runtime,
                pc: 0,
                line: 0,
                message: error.to_string(),
                traceback: Vec::new(),
            })?;
        let function_upvalues =
            self.heap
                .function_upvalues(*function)
                .map_err(|error| VmError {
                    kind: VmErrorKind::Runtime,
                    pc: 0,
                    line: 0,
                    message: error.to_string(),
                    traceback: Vec::new(),
                })?;
        let proto = proto_owner.as_ref();
        proto.validate_shape().map_err(|message| VmError {
            kind: VmErrorKind::Bytecode,
            pc: 0,
            line: 0,
            message: message.to_owned(),
            traceback: Vec::new(),
        })?;
        let (mut registers, varargs, mut top, mut pc) =
            if let Some((frame, resume_values)) = suspended {
                let SuspendedFrame {
                    registers,
                    varargs,
                    pc,
                    resume_mode,
                    ..
                } = frame;
                match resume_mode {
                    ResumeMode::Tail => return Ok(FrameOutcome::Return(resume_values)),
                    ResumeMode::Call { base, wanted } => {
                        let mut registers = registers;
                        let top = self.write_results(
                            &mut registers,
                            base,
                            &resume_values,
                            wanted,
                            proto,
                            pc.saturating_sub(1),
                        )?;
                        (registers, varargs, top, pc)
                    }
                }
            } else {
                let registers = (0..usize::from(proto.max_stack_size))
                    .map(|_| Rc::new(RefCell::new(Value::Nil)))
                    .collect::<Vec<StackSlot>>();
                for (index, argument) in arguments
                    .iter()
                    .take(usize::from(proto.num_params))
                    .enumerate()
                {
                    if let Some(register) = registers.get(index) {
                        *register.borrow_mut() = argument.clone();
                    }
                }
                let varargs = if proto.is_vararg {
                    arguments
                        .iter()
                        .skip(usize::from(proto.num_params))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                (
                    registers,
                    varargs,
                    usize::from(proto.num_params).min(usize::from(proto.max_stack_size)),
                    0,
                )
            };
        self.active_frames.insert(frame_id, registers.clone());
        while pc < proto.code.len() {
            if self.remaining_instructions == 0 {
                return Err(self.error_kind(
                    proto,
                    pc,
                    VmErrorKind::Limit,
                    "instruction limit exceeded",
                ));
            }
            self.remaining_instructions -= 1;
            let instruction = proto.code[pc];
            let opcode = instruction
                .opcode()
                .map_err(|raw| self.error(proto, pc, format!("invalid opcode {raw}")))?;
            if self.trace_enabled {
                let values = registers
                    .iter()
                    .map(|slot| {
                        slot.try_borrow()
                            .map(|value| value.clone())
                            .map_err(|_| self.error(proto, pc, "trace register borrow conflict"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.trace_events.push(VmTraceEvent {
                    frame_depth: self.call_stack.len(),
                    function: proto.debug_name.clone(),
                    pc,
                    line: proto.lines.get(pc).copied().unwrap_or(0),
                    opcode,
                    registers: values,
                });
            }
            let mut next_pc = pc + 1;

            match opcode {
                Opcode::LoadK => {
                    let value = proto
                        .constants
                        .get(instruction.bx() as usize)
                        .cloned()
                        .ok_or_else(|| self.error(proto, pc, "constant index is out of bounds"))?;
                    self.write(&registers, instruction.a(), value, proto, pc)?;
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
                    let value = self
                        .heap
                        .table_get(self.global_env, &Value::String(name.clone()))
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(&registers, instruction.a(), value, proto, pc)?;
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
                    self.heap
                        .table_set(self.global_env, Value::String(name.clone()), value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                // ANCHOR_END: globals
                Opcode::Move => {
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    self.write(&registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::Neg | Opcode::Not | Opcode::Len => {
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    let result = match opcode {
                        Opcode::Not => Value::Boolean(!value.is_truthy()),
                        Opcode::Neg => match value {
                            Value::Number(value) => Value::Number(-value),
                            other => {
                                let method = self
                                    .metamethod(&other, b"__unm", proto, pc)?
                                    .ok_or_else(|| {
                                        self.error_kind(
                                            proto,
                                            pc,
                                            VmErrorKind::Type,
                                            format!(
                                                "cannot negate {}, expected number",
                                                other.type_name()
                                            ),
                                        )
                                    })?;
                                self.call_event(method, vec![other.clone(), other], proto, pc)?
                            }
                        },
                        Opcode::Len => match value {
                            Value::String(value) => Value::Number(value.as_bytes().len() as f64),
                            Value::Table(value) => {
                                let object = Value::Table(value);
                                if let Some(method) =
                                    self.metamethod(&object, b"__len", proto, pc)?
                                {
                                    self.call_event(method, vec![object], proto, pc)?
                                } else {
                                    Value::Number(
                                        self.heap.table_array_len(value).map_err(|error| {
                                            self.error(proto, pc, error.to_string())
                                        })? as f64,
                                    )
                                }
                            }
                            other => {
                                let method = self
                                    .metamethod(&other, b"__len", proto, pc)?
                                    .ok_or_else(|| {
                                        self.error_kind(
                                            proto,
                                            pc,
                                            VmErrorKind::Type,
                                            format!("cannot get length of {}", other.type_name()),
                                        )
                                    })?;
                                self.call_event(method, vec![other], proto, pc)?
                            }
                        },
                        _ => unreachable!(),
                    };
                    self.write(&registers, instruction.a(), result, proto, pc)?;
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
                Opcode::JumpIfNil => {
                    if self.read(&registers, u16::from(instruction.a()), proto, pc)? == Value::Nil {
                        next_pc = self.jump_target(proto, pc, instruction.sbx())?;
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
                    self.write(&registers, base, Value::Number(initial - step), proto, pc)?;
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
                        self.write(&registers, base, Value::Number(next), proto, pc)?;
                        self.write(&registers, visible_register, Value::Number(next), proto, pc)?;
                        next_pc = self.jump_target(proto, pc, instruction.sbx())?;
                    }
                }
                // ANCHOR_END: loop_vm
                Opcode::Return => {
                    let first = usize::from(instruction.a());
                    let count = if instruction.b() == 0 {
                        top.saturating_sub(first)
                    } else {
                        usize::from(instruction.b() - 1)
                    };
                    let end = first.checked_add(count).ok_or_else(|| {
                        self.error(proto, pc, "return window arithmetic overflow")
                    })?;
                    if end > registers.len() {
                        return Err(self.error(proto, pc, "return window is out of bounds"));
                    }
                    return Ok(FrameOutcome::Return(
                        registers[first..end]
                            .iter()
                            .map(|slot| slot.borrow().clone())
                            .collect(),
                    ));
                }
                Opcode::Closure => {
                    let nested = proto
                        .protos
                        .get(instruction.bx() as usize)
                        .cloned()
                        .ok_or_else(|| {
                            self.error(proto, pc, "nested proto index is out of bounds")
                        })?;
                    let mut upvalues = Vec::with_capacity(nested.upvalues.len());
                    for descriptor in &nested.upvalues {
                        let upvalue = if descriptor.in_stack {
                            self.capture_upvalue(frame_id, descriptor.index, &registers, proto, pc)?
                        } else {
                            function_upvalues
                                .get(usize::from(descriptor.index))
                                .copied()
                                .ok_or_else(|| {
                                    self.error(proto, pc, "parent upvalue index is out of bounds")
                                })?
                        };
                        upvalues.push(upvalue);
                    }
                    let closure = self
                        .heap
                        .allocate_function_with_upvalues(nested, upvalues)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(
                        &registers,
                        instruction.a(),
                        Value::Function(closure),
                        proto,
                        pc,
                    )?;
                }
                Opcode::GetUpval => {
                    let upvalue = function_upvalues
                        .get(usize::from(instruction.b()))
                        .copied()
                        .ok_or_else(|| self.error(proto, pc, "upvalue index is out of bounds"))?;
                    let value = self
                        .heap
                        .upvalue_get(upvalue)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(&registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::SetUpval => {
                    let upvalue = function_upvalues
                        .get(usize::from(instruction.b()))
                        .copied()
                        .ok_or_else(|| self.error(proto, pc, "upvalue index is out of bounds"))?;
                    let value = self.read(&registers, u16::from(instruction.a()), proto, pc)?;
                    self.heap
                        .upvalue_set(upvalue, value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                Opcode::Close => {
                    self.close_upvalues(frame_id, instruction.a())
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    for slot in registers.iter().skip(usize::from(instruction.a())) {
                        *slot.borrow_mut() = Value::Nil;
                    }
                }
                Opcode::Call | Opcode::TailCall => {
                    let callable = self
                        .read(&registers, u16::from(instruction.a()), proto, pc)?
                        .clone();
                    let first = usize::from(instruction.a()) + 1;
                    let count = if instruction.b() == 0 {
                        top.saturating_sub(first)
                    } else {
                        usize::from(instruction.b() - 1)
                    };
                    let end = first.checked_add(count).ok_or_else(|| {
                        self.error(proto, pc, "call argument window arithmetic overflow")
                    })?;
                    if end > registers.len() {
                        return Err(self.error(proto, pc, "call argument window is out of bounds"));
                    }
                    let arguments = registers[first..end]
                        .iter()
                        .map(|slot| slot.borrow().clone())
                        .collect::<Vec<_>>();
                    let (callable, arguments, crossed_call_boundary) =
                        self.resolve_callable(callable, arguments, proto, pc)?;
                    let tail = opcode == Opcode::TailCall;
                    if tail && crossed_call_boundary {
                        let results =
                            self.invoke_yield_forbidden(callable, arguments, proto, pc)?;
                        return Ok(FrameOutcome::Return(results));
                    }
                    if tail {
                        match callable {
                            Value::Function(callee) => {
                                return Ok(FrameOutcome::TailCall(callee, arguments));
                            }
                            Value::NativeFunction(native) => {
                                match self.invoke_native(native, &arguments, proto, pc) {
                                    Ok(results) => return Ok(FrameOutcome::Return(results)),
                                    Err(error) if error.kind == VmErrorKind::Yield => {
                                        self.yield_frames.push(SuspendedFrame {
                                            function: *function,
                                            registers: registers.clone(),
                                            varargs: varargs.clone(),
                                            pc: next_pc,
                                            frame_id,
                                            resume_mode: ResumeMode::Tail,
                                        });
                                        return Err(error);
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            Value::CoroutineWrapper(coroutine) => {
                                let results = self.invoke(
                                    Value::CoroutineWrapper(coroutine),
                                    &arguments,
                                    proto,
                                    pc,
                                    first,
                                    end,
                                )?;
                                return Ok(FrameOutcome::Return(results));
                            }
                            other => {
                                return Err(self.error_kind(
                                    proto,
                                    pc,
                                    VmErrorKind::Type,
                                    format!("attempt to call a {} value", other.type_name()),
                                ));
                            }
                        }
                    }
                    let wanted = if instruction.c() == 0 {
                        None
                    } else {
                        Some(usize::from(instruction.c() - 1))
                    };
                    let invoked = if crossed_call_boundary {
                        self.invoke_yield_forbidden(callable, arguments, proto, pc)
                    } else {
                        self.invoke(callable, &arguments, proto, pc, first, end)
                    };
                    let results = match invoked {
                        Ok(results) => results,
                        Err(error) if error.kind == VmErrorKind::Yield => {
                            self.yield_frames.push(SuspendedFrame {
                                function: *function,
                                registers: registers.clone(),
                                varargs: varargs.clone(),
                                pc: next_pc,
                                frame_id,
                                resume_mode: ResumeMode::Call {
                                    base: usize::from(instruction.a()),
                                    wanted,
                                },
                            });
                            return Err(error);
                        }
                        Err(error) => return Err(error),
                    };
                    top = self.write_results(
                        &mut registers,
                        usize::from(instruction.a()),
                        &results,
                        wanted,
                        proto,
                        pc,
                    )?;
                    self.active_frames.insert(frame_id, registers.clone());
                }
                Opcode::Vararg => {
                    let wanted = if instruction.b() == 0 {
                        None
                    } else {
                        Some(usize::from(instruction.b() - 1))
                    };
                    top = self.write_results(
                        &mut registers,
                        usize::from(instruction.a()),
                        &varargs,
                        wanted,
                        proto,
                        pc,
                    )?;
                    self.active_frames.insert(frame_id, registers.clone());
                }
                // ANCHOR: table_vm
                Opcode::NewTable => {
                    let table = self
                        .heap
                        .allocate_table()
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(&registers, instruction.a(), Value::Table(table), proto, pc)?;
                }
                Opcode::GetTable => {
                    let table = self.read(&registers, instruction.b(), proto, pc)?;
                    let key = self.read(&registers, instruction.c(), proto, pc)?;
                    let value = self.index_value(table, key, proto, pc)?;
                    self.write(&registers, instruction.a(), value, proto, pc)?;
                }
                Opcode::SetTable => {
                    let table = self.read(&registers, u16::from(instruction.a()), proto, pc)?;
                    let key = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    let value = self.read(&registers, instruction.c(), proto, pc)?.clone();
                    self.set_indexed(table, key, value, proto, pc)?;
                }
                Opcode::SetList => {
                    let table =
                        self.table_register(&registers, u16::from(instruction.a()), proto, pc)?;
                    let value = self.read(&registers, instruction.b(), proto, pc)?.clone();
                    self.heap
                        .table_set(table, Value::Number(f64::from(instruction.c())), value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                Opcode::SetListOpen => {
                    let table =
                        self.table_register(&registers, u16::from(instruction.a()), proto, pc)?;
                    let first = usize::from(instruction.b());
                    let start = usize::from(instruction.c());
                    if first > top || top > registers.len() {
                        return Err(self.error(
                            proto,
                            pc,
                            "open table value window is out of bounds",
                        ));
                    }
                    for (offset, value) in registers[first..top]
                        .iter()
                        .map(|slot| slot.borrow().clone())
                        .enumerate()
                    {
                        self.heap
                            .table_set(table, Value::Number((start + offset) as f64), value)
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    }
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
                    self.write(&registers, instruction.a(), result, proto, pc)?;
                }
            }
            if !self.manual_gc_mode && self.heap.checkpoint_due() {
                self.gc_step_internal(1, proto, pc)?;
            }
            pc = next_pc;
        }
        Err(self.error(proto, pc.saturating_sub(1), "proto ended without RETURN"))
    }
    // ANCHOR_END: call_frame
    // ANCHOR_END: dispatch

    // ANCHOR: variable_results
    fn invoke(
        &mut self,
        function: Value,
        arguments: &[Value],
        caller: &Proto,
        pc: usize,
        base: usize,
        top: usize,
    ) -> Result<Vec<Value>, VmError> {
        let root_base = self.temporary_roots.len();
        self.temporary_roots.push(function.clone());
        self.temporary_roots.extend_from_slice(arguments);
        let result = match function {
            Value::Function(function) => {
                if self.call_stack.len() >= self.max_call_depth {
                    Err(self.error_kind(
                        caller,
                        pc,
                        VmErrorKind::Limit,
                        "call depth limit exceeded",
                    ))
                } else {
                    self.call_stack.push(CallInfo {
                        source: caller.debug_name.clone(),
                        line: caller.lines.get(pc).copied().unwrap_or(0),
                        base,
                        top,
                    });
                    let result = self.execute_proto(function, arguments);
                    self.call_stack.pop();
                    result
                }
            }
            Value::NativeFunction(function) => self.invoke_native(function, arguments, caller, pc),
            Value::CoroutineWrapper(coroutine) => {
                let mut results = self.resume_coroutine(coroutine, arguments, caller, pc)?;
                let succeeded = matches!(results.first(), Some(Value::Boolean(true)));
                if succeeded {
                    results.remove(0);
                    Ok(results)
                } else {
                    let message = results
                        .get(1)
                        .map(Self::stringify)
                        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        .unwrap_or_else(|| "cannot resume coroutine".to_owned());
                    Err(self.error_kind(caller, pc, VmErrorKind::Runtime, message))
                }
            }
            other => Err(self.error_kind(
                caller,
                pc,
                VmErrorKind::Type,
                format!("attempt to call a {} value", other.type_name()),
            )),
        };
        self.temporary_roots.truncate(root_base);
        result
    }

    fn invoke_native(
        &mut self,
        function: NativeFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        match function {
            NativeFunction::Base(function) => self.invoke_base(function, arguments, proto, pc),
            NativeFunction::Math(function) => self.invoke_math(function, arguments, proto, pc),
            NativeFunction::String(function) => self.invoke_string(function, arguments, proto, pc),
            NativeFunction::Table(function) => self.invoke_table(function, arguments, proto, pc),
            NativeFunction::Io(function) => self.invoke_io(function, arguments, proto, pc),
            NativeFunction::File(function) => self.invoke_file(function, arguments, proto, pc),
            NativeFunction::Os(function) => self.invoke_os(function, arguments, proto, pc),
            NativeFunction::Debug(function) => self.invoke_debug(function, arguments, proto, pc),
            NativeFunction::Package(function) => {
                self.invoke_package(function, arguments, proto, pc)
            }
            NativeFunction::Next => {
                let table = Self::native_table(arguments.first(), "next")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let after = arguments.get(1).cloned().unwrap_or(Value::Nil);
                Ok(
                    match self.heap.table_next_pair(table, &after).map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string())
                    })? {
                        Some((key, value)) => vec![key, value],
                        None => vec![Value::Nil],
                    },
                )
            }
            NativeFunction::Pairs => {
                let table = Self::native_table(arguments.first(), "pairs")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                Ok(vec![
                    Value::NativeFunction(NativeFunction::Next),
                    Value::Table(table),
                    Value::Nil,
                ])
            }
            NativeFunction::IPairs => {
                let table = Self::native_table(arguments.first(), "ipairs")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                Ok(vec![
                    Value::NativeFunction(NativeFunction::IPairsAux),
                    Value::Table(table),
                    Value::Number(0.0),
                ])
            }
            NativeFunction::IPairsAux => {
                let table = Self::native_table(arguments.first(), "ipairs iterator")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let index = match arguments.get(1) {
                    Some(Value::Number(index)) => *index,
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "ipairs iterator index must be number, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                    None => 0.0,
                } + 1.0;
                let value = self
                    .heap
                    .table_get(table, &Value::Number(index))
                    .map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string())
                    })?;
                if value == Value::Nil {
                    Ok(vec![Value::Nil])
                } else {
                    Ok(vec![Value::Number(index), value])
                }
            }
            NativeFunction::SetMetatable => {
                let table = Self::native_table(arguments.first(), "setmetatable")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                if let Some(current) = self.heap.table_metatable(table).map_err(|error| {
                    self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                })? {
                    if self
                        .heap
                        .table_get(
                            current,
                            &Value::String(ByteString::new(&b"__metatable"[..])),
                        )
                        .map_err(|error| {
                            self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                        })?
                        != Value::Nil
                    {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Metamethod,
                            "cannot change a protected metatable",
                        ));
                    }
                }
                let metatable = match arguments.get(1) {
                    Some(Value::Table(metatable)) => Some(*metatable),
                    Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "setmetatable expected table or nil, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                };
                self.heap
                    .table_set_metatable(table, metatable)
                    .map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                    })?;
                Ok(vec![Value::Table(table)])
            }
            NativeFunction::GetMetatable => {
                let value = arguments.first().cloned().unwrap_or(Value::Nil);
                let Some(metatable) = self.metatable_for(&value, proto, pc)? else {
                    return Ok(vec![Value::Nil]);
                };
                let protected = self
                    .heap
                    .table_get(
                        metatable,
                        &Value::String(ByteString::new(&b"__metatable"[..])),
                    )
                    .map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                    })?;
                Ok(vec![if protected == Value::Nil {
                    Value::Table(metatable)
                } else {
                    protected
                }])
            }
            NativeFunction::RawGet => {
                let table = Self::native_table(arguments.first(), "rawget")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let key = arguments.get(1).cloned().unwrap_or(Value::Nil);
                Ok(vec![self.heap.table_get(table, &key).map_err(|error| {
                    self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string())
                })?])
            }
            NativeFunction::RawSet => {
                let table = Self::native_table(arguments.first(), "rawset")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let key = arguments.get(1).cloned().unwrap_or(Value::Nil);
                let value = arguments.get(2).cloned().unwrap_or(Value::Nil);
                self.heap.table_set(table, key, value).map_err(|error| {
                    self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string())
                })?;
                Ok(vec![Value::Table(table)])
            }
            NativeFunction::ToString => {
                let value = arguments.first().cloned().unwrap_or(Value::Nil);
                if let Some(method) = self.metamethod(&value, b"__tostring", proto, pc)? {
                    let result = self.call_event(method, vec![value], proto, pc)?;
                    if !matches!(result, Value::String(_)) {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Metamethod,
                            "__tostring must return a string",
                        ));
                    }
                    return Ok(vec![result]);
                }
                Ok(vec![Value::String(ByteString::new(Self::stringify(
                    &value,
                )))])
            }
            NativeFunction::SetTypeMetatable => {
                let Some(Value::String(name)) = arguments.first() else {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "settypemetatable expected a type-name string",
                    ));
                };
                let Some(type_name) = Self::supported_type_name(name.as_bytes()) else {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        format!("unsupported primitive type name '{name}'"),
                    ));
                };
                let metatable = match arguments.get(1) {
                    Some(Value::Table(table)) => Some(*table),
                    Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "settypemetatable expected table or nil, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                };
                if let Some(metatable) = metatable {
                    self.type_metatables.insert(type_name, metatable);
                    Ok(vec![Value::Table(metatable)])
                } else {
                    self.type_metatables.remove(type_name);
                    Ok(vec![Value::Nil])
                }
            }
            NativeFunction::GcCollect => {
                let stats = self.finish_gc(proto, pc)?;
                self.manual_gc_mode = false;
                Ok(vec![Value::Number(stats.collected as f64)])
            }
            NativeFunction::GcLive => Ok(vec![Value::Number(self.heap.live_count() as f64)]),
            NativeFunction::GcStep => {
                let budget = match arguments.first() {
                    Some(Value::Number(value)) if value.is_finite() && *value >= 1.0 => {
                        *value as usize
                    }
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "gc_step expected positive number, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                    None => 1,
                };
                self.manual_gc_mode = true;
                self.gc_step_internal(budget, proto, pc)?;
                let completed = self.heap.phase() == HeapPhase::Pause;
                self.manual_gc_mode = !completed;
                Ok(vec![Value::Boolean(completed)])
            }
            NativeFunction::GcPhase => Ok(vec![Value::String(ByteString::new(
                match self.heap.phase() {
                    HeapPhase::Pause => b"pause".as_slice(),
                    HeapPhase::Mark => b"mark".as_slice(),
                    HeapPhase::Atomic => b"atomic".as_slice(),
                    HeapPhase::Sweep => b"sweep".as_slice(),
                },
            ))]),
            NativeFunction::NewUserdata => {
                let userdata = self
                    .heap
                    .allocate_userdata()
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Userdata(userdata)])
            }
            NativeFunction::SetFinalizer => {
                let userdata = Self::native_userdata(arguments.first(), "setfinalizer")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let finalizer = match arguments.get(1) {
                    Some(Value::Nil) | None => None,
                    Some(value) => Some(value.clone()),
                };
                self.heap
                    .userdata_set_finalizer(userdata, finalizer)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Userdata(userdata)])
            }
            NativeFunction::UserdataId => {
                let userdata = Self::native_userdata(arguments.first(), "userdataid")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                Ok(vec![Value::Number(userdata.object_id() as f64)])
            }
            NativeFunction::CoroutineCreate => {
                let function = match arguments.first() {
                    Some(
                        value @ (Value::Function(_)
                        | Value::NativeFunction(_)
                        | Value::CoroutineWrapper(_)),
                    ) => value.clone(),
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "coroutine.create expected function, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                    None => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "coroutine.create expected function, got no value",
                        ));
                    }
                };
                Ok(vec![Value::Coroutine(
                    self.create_coroutine(function, proto, pc)?,
                )])
            }
            NativeFunction::CoroutineResume => {
                let coroutine = Self::native_coroutine(arguments.first(), "coroutine.resume")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                self.resume_coroutine(coroutine, &arguments[1..], proto, pc)
            }
            NativeFunction::CoroutineYield => {
                if self.current_coroutine.is_none() {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "attempt to yield from the main thread",
                    ));
                }
                if self.yield_forbidden_depth != 0 {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "attempt to yield across a metamethod or finalizer boundary",
                    ));
                }
                self.yield_values = Some(arguments.to_vec());
                Err(self.error_kind(proto, pc, VmErrorKind::Yield, "coroutine yielded"))
            }
            NativeFunction::CoroutineStatus => {
                let coroutine = Self::native_coroutine(arguments.first(), "coroutine.status")
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                let status = self.coroutines.get(&coroutine).ok_or_else(|| {
                    self.error_kind(proto, pc, VmErrorKind::Runtime, "unknown coroutine handle")
                })?;
                Ok(vec![Value::String(ByteString::new(status.status.name()))])
            }
            NativeFunction::CoroutineRunning => Ok(vec![self
                .current_coroutine
                .map(Value::Coroutine)
                .unwrap_or(Value::Nil)]),
            NativeFunction::CoroutineWrap => {
                let function = match arguments.first() {
                    Some(
                        value @ (Value::Function(_)
                        | Value::NativeFunction(_)
                        | Value::CoroutineWrapper(_)),
                    ) => value.clone(),
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "coroutine.wrap expected function, got {}",
                                other.type_name()
                            ),
                        ));
                    }
                    None => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "coroutine.wrap expected function, got no value",
                        ));
                    }
                };
                Ok(vec![Value::CoroutineWrapper(
                    self.create_coroutine(function, proto, pc)?,
                )])
            }
        }
    }

    // ANCHOR: stdlib_adapter
    fn invoke_base(
        &mut self,
        function: BaseFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        match function {
            BaseFunction::Assert => {
                if args.get(0).is_some_and(Value::is_truthy) {
                    Ok(arguments.to_vec())
                } else {
                    let message = args
                        .get(1)
                        .map(Self::stringify)
                        .unwrap_or_else(|| b"assertion failed!".to_vec());
                    Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Runtime,
                        String::from_utf8_lossy(&message).into_owned(),
                    ))
                }
            }
            BaseFunction::Error => {
                let message = args.get(0).map(Self::stringify).unwrap_or_default();
                Err(self.error_kind(
                    proto,
                    pc,
                    VmErrorKind::Runtime,
                    String::from_utf8_lossy(&message).into_owned(),
                ))
            }
            BaseFunction::Type => Ok(vec![Value::String(ByteString::new(
                args.required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    .type_name()
                    .as_bytes(),
            ))]),
            BaseFunction::ToNumber => {
                let Some(value) = args.get(0) else {
                    return Ok(vec![Value::Nil]);
                };
                let base = match args.get(1) {
                    None | Some(Value::Nil) => None,
                    Some(_) => Some(
                        args.integer(1)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                    ),
                };
                let number = match (value, base) {
                    (Value::Number(number), None) => Some(*number),
                    (Value::String(bytes), None) => std::str::from_utf8(bytes.as_bytes())
                        .ok()
                        .and_then(|text| text.trim().parse::<f64>().ok()),
                    (Value::String(bytes), Some(base @ 2..=36)) => {
                        Self::parse_integer_base(bytes.as_bytes(), base as u32)
                    }
                    (_, Some(base)) if !(2..=36).contains(&base) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "bad argument #2 to 'tonumber' (base out of range)",
                        ));
                    }
                    _ => None,
                };
                Ok(vec![number.map(Value::Number).unwrap_or(Value::Nil)])
            }
            BaseFunction::RawEqual => Ok(vec![Value::Boolean(
                args.get(0).cloned().unwrap_or(Value::Nil)
                    == args.get(1).cloned().unwrap_or(Value::Nil),
            )]),
            BaseFunction::Select => {
                let selector = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                if matches!(selector, Value::String(bytes) if bytes.as_bytes() == b"#") {
                    return Ok(vec![
                        Value::Number(arguments.len().saturating_sub(1) as f64),
                    ]);
                }
                let mut index = args
                    .integer(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let count = arguments.len().saturating_sub(1) as i64;
                if index < 0 {
                    index += count + 1;
                }
                if index == 0 || index < -count {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "bad argument #1 to 'select' (index out of range)",
                    ));
                }
                let start = usize::try_from(index.max(1)).unwrap_or(usize::MAX);
                Ok(arguments.get(start..).unwrap_or_default().to_vec())
            }
            BaseFunction::Unpack => {
                let table = args
                    .table(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let start = args
                    .optional_integer(1, 1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let default_end = self
                    .heap
                    .table_array_len(table)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?
                    as i64;
                let end = args
                    .optional_integer(2, default_end)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                if end < start {
                    return Ok(Vec::new());
                }
                if end.saturating_sub(start) >= 255 {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Limit,
                        "too many results to unpack",
                    ));
                }
                let mut results = Vec::new();
                for index in start..=end {
                    results.push(
                        self.heap
                            .table_get(table, &Value::Number(index as f64))
                            .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    );
                }
                Ok(results)
            }
            BaseFunction::PCall => {
                let callable = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    .clone();
                match self.invoke_yield_forbidden(callable, args.tail(1).to_vec(), proto, pc) {
                    Ok(values) => {
                        let mut results = vec![Value::Boolean(true)];
                        results.extend(values);
                        Ok(results)
                    }
                    Err(error) => Ok(vec![
                        Value::Boolean(false),
                        Value::String(ByteString::new(error.to_string().as_bytes())),
                    ]),
                }
            }
            BaseFunction::XPCall => {
                let callable = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    .clone();
                let handler = args
                    .required(1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    .clone();
                match self.invoke_yield_forbidden(callable, Vec::new(), proto, pc) {
                    Ok(values) => {
                        let mut results = vec![Value::Boolean(true)];
                        results.extend(values);
                        Ok(results)
                    }
                    Err(error) => {
                        let message = Value::String(ByteString::new(error.to_string().as_bytes()));
                        let handled = self
                            .invoke_yield_forbidden(handler, vec![message], proto, pc)
                            .unwrap_or_else(|handler_error| {
                                vec![Value::String(ByteString::new(
                                    handler_error.to_string().as_bytes(),
                                ))]
                            });
                        Ok(vec![
                            Value::Boolean(false),
                            handled.into_iter().next().unwrap_or(Value::Nil),
                        ])
                    }
                }
            }
            BaseFunction::Print => {
                let line = arguments
                    .iter()
                    .map(Self::stringify)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .collect::<Vec<_>>()
                    .join("\t");
                println!("{line}");
                Ok(Vec::new())
            }
            BaseFunction::CollectGarbage => {
                let option = match args.get(0) {
                    None | Some(Value::Nil) => b"collect".to_vec(),
                    Some(_) => args
                        .bytes(0)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                match option.as_slice() {
                    b"collect" => {
                        self.finish_gc(proto, pc)?;
                        Ok(vec![Value::Number(0.0)])
                    }
                    b"count" => Ok(vec![Value::Number(self.heap.live_count() as f64)]),
                    b"step" => {
                        let budget = args
                            .optional_integer(1, 1)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                            .max(1) as usize;
                        let completed = self
                            .gc_step_internal(budget, proto, pc)?
                            .completed
                            .is_some();
                        Ok(vec![Value::Boolean(completed)])
                    }
                    b"stop" => {
                        self.manual_gc_mode = true;
                        Ok(vec![Value::Number(0.0)])
                    }
                    b"restart" => {
                        self.manual_gc_mode = false;
                        Ok(vec![Value::Number(0.0)])
                    }
                    b"setpause" | b"setstepmul" => Ok(vec![Value::Number(0.0)]),
                    _ => Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "bad argument #1 to 'collectgarbage' (invalid option)",
                    )),
                }
            }
            BaseFunction::LoadString => {
                let source = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                match self.compile_source(&source, proto, pc) {
                    Ok(function) => Ok(vec![Value::Function(function)]),
                    Err(error) => Ok(vec![
                        Value::Nil,
                        Value::String(ByteString::new(error.to_string().as_bytes())),
                    ]),
                }
            }
            BaseFunction::LoadFile => {
                let path = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                match self.load_file_function(&path, proto, pc) {
                    Ok(function) => Ok(vec![Value::Function(function)]),
                    Err(error) => Ok(vec![
                        Value::Nil,
                        Value::String(ByteString::new(error.to_string().as_bytes())),
                    ]),
                }
            }
            BaseFunction::DoFile => {
                let path = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let function = self.load_file_function(&path, proto, pc)?;
                self.invoke(Value::Function(function), &[], proto, pc, 0, 0)
            }
        }
    }

    fn invoke_math(
        &mut self,
        function: MathFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        let number = |index| {
            args.number(index)
                .map_err(|error| self.host_error(function.name(), error, proto, pc))
        };
        let unary = |operation: fn(f64) -> f64| {
            number(0).map(|value| vec![Value::Number(operation(value))])
        };
        match function {
            MathFunction::Abs => unary(f64::abs),
            MathFunction::Acos => unary(f64::acos),
            MathFunction::Asin => unary(f64::asin),
            MathFunction::Atan => unary(f64::atan),
            MathFunction::Ceil => unary(f64::ceil),
            MathFunction::Cos => unary(f64::cos),
            MathFunction::Cosh => unary(f64::cosh),
            MathFunction::Exp => unary(f64::exp),
            MathFunction::Floor => unary(f64::floor),
            MathFunction::Log => unary(f64::ln),
            MathFunction::Log10 => unary(f64::log10),
            MathFunction::Sin => unary(f64::sin),
            MathFunction::Sinh => unary(f64::sinh),
            MathFunction::Sqrt => unary(f64::sqrt),
            MathFunction::Tan => unary(f64::tan),
            MathFunction::Tanh => unary(f64::tanh),
            MathFunction::Deg => Ok(vec![Value::Number(number(0)?.to_degrees())]),
            MathFunction::Rad => Ok(vec![Value::Number(number(0)?.to_radians())]),
            MathFunction::Atan2 => Ok(vec![Value::Number(number(0)?.atan2(number(1)?))]),
            MathFunction::Fmod => Ok(vec![Value::Number(number(0)? % number(1)?)]),
            MathFunction::Ldexp => Ok(vec![Value::Number(number(0)? * 2.0_f64.powf(number(1)?))]),
            MathFunction::Pow => Ok(vec![Value::Number(number(0)?.powf(number(1)?))]),
            MathFunction::Frexp => {
                let value = number(0)?;
                if value == 0.0 || !value.is_finite() {
                    Ok(vec![Value::Number(value), Value::Number(0.0)])
                } else {
                    let exponent = value.abs().log2().floor() + 1.0;
                    Ok(vec![
                        Value::Number(value / 2.0_f64.powf(exponent)),
                        Value::Number(exponent),
                    ])
                }
            }
            MathFunction::Modf => {
                let value = number(0)?;
                let integer = value.trunc();
                Ok(vec![Value::Number(integer), Value::Number(value - integer)])
            }
            MathFunction::Max | MathFunction::Min => {
                if args.is_empty() {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        format!("bad argument #1 to '{}' (number expected)", function.name()),
                    ));
                }
                let mut result = number(0)?;
                for index in 1..args.len() {
                    let value = number(index)?;
                    result = if function == MathFunction::Max {
                        result.max(value)
                    } else {
                        result.min(value)
                    };
                }
                Ok(vec![Value::Number(result)])
            }
            MathFunction::RandomSeed => {
                self.random_state = args
                    .integer(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    as u64;
                Ok(Vec::new())
            }
            MathFunction::Random => {
                self.random_state = self
                    .random_state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let fraction = ((self.random_state >> 11) as f64) / ((1_u64 << 53) as f64);
                match args.len() {
                    0 => Ok(vec![Value::Number(fraction)]),
                    1 => {
                        let upper = args
                            .integer(0)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                        if upper < 1 {
                            return Err(self.error_kind(
                                proto,
                                pc,
                                VmErrorKind::Type,
                                "bad argument #1 to 'math.random' (interval is empty)",
                            ));
                        }
                        Ok(vec![Value::Number((fraction * upper as f64).floor() + 1.0)])
                    }
                    2 => {
                        let lower = args
                            .integer(0)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                        let upper = args
                            .integer(1)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                        if lower > upper {
                            return Err(self.error_kind(
                                proto,
                                pc,
                                VmErrorKind::Type,
                                "bad argument #2 to 'math.random' (interval is empty)",
                            ));
                        }
                        let width = upper.saturating_sub(lower).saturating_add(1) as f64;
                        Ok(vec![Value::Number(
                            lower as f64 + (fraction * width).floor(),
                        )])
                    }
                    _ => Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "wrong number of arguments to 'math.random'",
                    )),
                }
            }
        }
    }

    fn create_coroutine(
        &mut self,
        function: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<CoroutineRef, VmError> {
        let coroutine = self
            .heap
            .allocate_coroutine(vec![function.clone()])
            .map_err(|error| self.error(proto, pc, error.to_string()))?;
        self.coroutines.insert(
            coroutine,
            CoroutineState {
                status: CoroutineStatus::Suspended,
                entry: Some(function),
                frames: Vec::new(),
            },
        );
        Ok(coroutine)
    }

    // ANCHOR: coroutine_resume
    fn resume_coroutine(
        &mut self,
        coroutine: CoroutineRef,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        if !self.coroutines.contains_key(&coroutine) {
            return Err(self.error_kind(
                proto,
                pc,
                VmErrorKind::Runtime,
                "unknown coroutine handle",
            ));
        }
        let (entry, mut frames) = {
            let state = self
                .coroutines
                .get_mut(&coroutine)
                .expect("state checked above");
            if state.status != CoroutineStatus::Suspended {
                return Ok(vec![
                    Value::Boolean(false),
                    Value::String(ByteString::new(
                        &b"cannot resume non-suspended coroutine"[..],
                    )),
                ]);
            }
            state.status = CoroutineStatus::Running;
            (state.entry.take(), std::mem::take(&mut state.frames))
        };

        let parent = self.current_coroutine;
        if let Some(parent) = parent {
            if let Some(state) = self.coroutines.get_mut(&parent) {
                state.status = CoroutineStatus::Normal;
            }
        }
        let outer_yield_values = self.yield_values.take();
        let outer_yield_frames = std::mem::take(&mut self.yield_frames);
        self.current_coroutine = Some(coroutine);

        let result = if let Some(function) = entry {
            match function {
                Value::Function(function) => self.execute_proto(function, arguments),
                Value::NativeFunction(function) => {
                    self.yield_forbidden_depth += 1;
                    let result = self.invoke_native(function, arguments, proto, pc);
                    self.yield_forbidden_depth -= 1;
                    result
                }
                Value::CoroutineWrapper(coroutine) => self.invoke(
                    Value::CoroutineWrapper(coroutine),
                    arguments,
                    proto,
                    pc,
                    0,
                    arguments.len(),
                ),
                _ => unreachable!("coroutine entry is validated at creation"),
            }
        } else {
            let mut resume_values = arguments.to_vec();
            let mut final_result = None;
            while !frames.is_empty() {
                let frame = frames.remove(0);
                let outcome = match self.resume_frame(frame, resume_values) {
                    Ok(FrameOutcome::Return(values)) => Ok(values),
                    Ok(FrameOutcome::TailCall(function, arguments)) => {
                        self.execute_proto(function, &arguments)
                    }
                    Err(error) => Err(error),
                };
                match outcome {
                    Ok(values) if !frames.is_empty() => resume_values = values,
                    outcome => {
                        final_result = Some(outcome);
                        break;
                    }
                }
            }
            final_result.unwrap_or_else(|| {
                Err(self.error_kind(
                    proto,
                    pc,
                    VmErrorKind::Runtime,
                    "suspended coroutine has no saved frame",
                ))
            })
        };

        let mut new_frames = std::mem::take(&mut self.yield_frames);
        let yielded_values = self.yield_values.take();
        self.current_coroutine = parent;
        if let Some(parent) = parent {
            if let Some(state) = self.coroutines.get_mut(&parent) {
                state.status = CoroutineStatus::Running;
            }
        }
        self.yield_values = outer_yield_values;
        self.yield_frames = outer_yield_frames;

        match result {
            Ok(values) => {
                let state = self
                    .coroutines
                    .get_mut(&coroutine)
                    .expect("running state exists");
                state.status = CoroutineStatus::Dead;
                state.frames.clear();
                self.heap
                    .coroutine_set_roots(coroutine, Vec::new())
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                let mut results = vec![Value::Boolean(true)];
                results.extend(values);
                Ok(results)
            }
            Err(error) if error.kind == VmErrorKind::Yield => {
                new_frames.extend(frames);
                let state = self
                    .coroutines
                    .get_mut(&coroutine)
                    .expect("running state exists");
                state.status = CoroutineStatus::Suspended;
                state.frames = new_frames;
                self.sync_coroutine_roots(coroutine, proto, pc)?;
                let mut results = vec![Value::Boolean(true)];
                results.extend(yielded_values.unwrap_or_default());
                Ok(results)
            }
            Err(error) => {
                self.close_suspended_frames(&frames, proto, pc)?;
                self.close_suspended_frames(&new_frames, proto, pc)?;
                let state = self
                    .coroutines
                    .get_mut(&coroutine)
                    .expect("running state exists");
                state.status = CoroutineStatus::Dead;
                state.frames.clear();
                self.heap
                    .coroutine_set_roots(coroutine, Vec::new())
                    .map_err(|cleanup| self.error(proto, pc, cleanup.to_string()))?;
                Ok(vec![
                    Value::Boolean(false),
                    Value::String(ByteString::new(error.to_string().as_bytes())),
                ])
            }
        }
    }
    // ANCHOR_END: coroutine_resume

    fn sync_coroutine_roots(
        &mut self,
        coroutine: CoroutineRef,
        proto: &Proto,
        pc: usize,
    ) -> Result<(), VmError> {
        let state = self
            .coroutines
            .get(&coroutine)
            .expect("coroutine state exists");
        let mut roots = state.entry.iter().cloned().collect::<Vec<_>>();
        for frame in &state.frames {
            roots.push(Value::Function(frame.function));
            roots.extend(frame.varargs.iter().cloned());
            for slot in &frame.registers {
                roots.push(
                    slot.try_borrow()
                        .map_err(|_| self.error(proto, pc, "coroutine register borrow conflict"))?
                        .clone(),
                );
            }
        }
        self.heap
            .coroutine_set_roots(coroutine, roots)
            .map_err(|error| self.error(proto, pc, error.to_string()))
    }

    fn close_suspended_frames(
        &mut self,
        frames: &[SuspendedFrame],
        proto: &Proto,
        pc: usize,
    ) -> Result<(), VmError> {
        for frame in frames {
            self.close_upvalues(frame.frame_id, 0)
                .map_err(|error| self.error(proto, pc, error.to_string()))?;
        }
        Ok(())
    }

    fn prune_collected_coroutines(&mut self) {
        let collected = self
            .coroutines
            .keys()
            .copied()
            .filter(|coroutine| matches!(self.heap.is_live(*coroutine), Ok(false)))
            .collect::<Vec<_>>();
        for coroutine in collected {
            if let Some(state) = self.coroutines.remove(&coroutine) {
                let frame_ids = state
                    .frames
                    .iter()
                    .map(|frame| frame.frame_id)
                    .collect::<Vec<_>>();
                self.open_upvalues
                    .retain(|(owner, _), _| !frame_ids.contains(owner));
            }
        }
    }

    fn invoke_string(
        &mut self,
        function: StringFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        let bytes = |index| {
            args.bytes(index)
                .map_err(|error| self.host_error(function.name(), error, proto, pc))
        };
        let integer = |index| {
            args.integer(index)
                .map_err(|error| self.host_error(function.name(), error, proto, pc))
        };
        match function {
            StringFunction::Len => Ok(vec![Value::Number(bytes(0)?.len() as f64)]),
            StringFunction::Lower => Ok(vec![Value::String(ByteString::new(
                bytes(0)?
                    .into_iter()
                    .map(|byte| byte.to_ascii_lowercase())
                    .collect::<Vec<_>>(),
            ))]),
            StringFunction::Upper => Ok(vec![Value::String(ByteString::new(
                bytes(0)?
                    .into_iter()
                    .map(|byte| byte.to_ascii_uppercase())
                    .collect::<Vec<_>>(),
            ))]),
            StringFunction::Reverse => {
                let mut value = bytes(0)?;
                value.reverse();
                Ok(vec![Value::String(ByteString::new(value))])
            }
            StringFunction::Rep => {
                let value = bytes(0)?;
                let count = integer(1)?;
                if count <= 0 {
                    return Ok(vec![Value::String(ByteString::new(&b""[..]))]);
                }
                let count = usize::try_from(count).map_err(|_| {
                    self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Limit,
                        "string.rep result is too large",
                    )
                })?;
                let capacity = value.len().checked_mul(count).ok_or_else(|| {
                    self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Limit,
                        "string.rep result is too large",
                    )
                })?;
                let mut result = Vec::with_capacity(capacity);
                for _ in 0..count {
                    result.extend_from_slice(&value);
                }
                Ok(vec![Value::String(ByteString::new(result))])
            }
            StringFunction::Sub => {
                let value = bytes(0)?;
                let start = Self::string_index(integer(1)?, value.len());
                let end = Self::string_index(
                    args.optional_integer(2, -1)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                    value.len(),
                );
                let start = start.max(1) as usize;
                let end = end.min(value.len() as i64).max(0) as usize;
                let slice = if start > end || start > value.len() {
                    &[][..]
                } else {
                    &value[start - 1..end]
                };
                Ok(vec![Value::String(ByteString::new(slice))])
            }
            StringFunction::Byte => {
                let value = bytes(0)?;
                let start_raw = args
                    .optional_integer(1, 1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let end_raw = args
                    .optional_integer(2, start_raw)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let start = Self::string_index(start_raw, value.len()).max(1) as usize;
                let end = Self::string_index(end_raw, value.len())
                    .min(value.len() as i64)
                    .max(0) as usize;
                if start > end || start > value.len() {
                    return Ok(Vec::new());
                }
                Ok(value[start - 1..end]
                    .iter()
                    .map(|byte| Value::Number(f64::from(*byte)))
                    .collect())
            }
            StringFunction::Char => {
                let mut result = Vec::with_capacity(args.len());
                for index in 0..args.len() {
                    let value = integer(index)?;
                    let byte = u8::try_from(value).map_err(|_| {
                        self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "bad argument #{} to 'string.char' (value out of range)",
                                index + 1
                            ),
                        )
                    })?;
                    result.push(byte);
                }
                Ok(vec![Value::String(ByteString::new(result))])
            }
            StringFunction::Find => {
                let haystack = bytes(0)?;
                let needle = bytes(1)?;
                let start_raw = args
                    .optional_integer(2, 1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let start = Self::string_index(start_raw, haystack.len()).max(1) as usize;
                if start > haystack.len().saturating_add(1) {
                    return Ok(vec![Value::Nil]);
                }
                let offset = if needle.is_empty() {
                    Some(0)
                } else {
                    haystack[start - 1..]
                        .windows(needle.len())
                        .position(|window| window == needle)
                };
                Ok(match offset {
                    Some(offset) => {
                        let first = start + offset;
                        let last = if needle.is_empty() {
                            first.saturating_sub(1)
                        } else {
                            first + needle.len() - 1
                        };
                        vec![Value::Number(first as f64), Value::Number(last as f64)]
                    }
                    None => vec![Value::Nil],
                })
            }
            StringFunction::Format => {
                let format = bytes(0)?;
                let mut output = Vec::new();
                let mut argument = 1;
                let mut cursor = 0;
                while cursor < format.len() {
                    if format[cursor] != b'%' {
                        output.push(format[cursor]);
                        cursor += 1;
                        continue;
                    }
                    cursor += 1;
                    let Some(specifier) = format.get(cursor).copied() else {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "invalid format (ends with '%')",
                        ));
                    };
                    cursor += 1;
                    if specifier == b'%' {
                        output.push(b'%');
                        continue;
                    }
                    let value = args
                        .required(argument)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                    match specifier {
                        b's' => output.extend(Self::stringify(value)),
                        b'd' | b'i' => output.extend(
                            args.integer(argument)
                                .map_err(|error| {
                                    self.host_error(function.name(), error, proto, pc)
                                })?
                                .to_string()
                                .as_bytes(),
                        ),
                        b'f' => output.extend(
                            args.number(argument)
                                .map_err(|error| {
                                    self.host_error(function.name(), error, proto, pc)
                                })?
                                .to_string()
                                .as_bytes(),
                        ),
                        b'q' => {
                            output.push(b'"');
                            for byte in Args::new(std::slice::from_ref(value)).bytes(0).map_err(
                                |error| self.host_error(function.name(), error, proto, pc),
                            )? {
                                match byte {
                                    b'"' | b'\\' => {
                                        output.push(b'\\');
                                        output.push(byte);
                                    }
                                    b'\n' => output.extend_from_slice(b"\\n"),
                                    b'\r' => output.extend_from_slice(b"\\r"),
                                    0 => output.extend_from_slice(b"\\000"),
                                    _ => output.push(byte),
                                }
                            }
                            output.push(b'"');
                        }
                        _ => {
                            return Err(self.error_kind(
                                proto,
                                pc,
                                VmErrorKind::Type,
                                format!("unsupported format option '%{}'", specifier as char),
                            ));
                        }
                    }
                    argument += 1;
                }
                Ok(vec![Value::String(ByteString::new(output))])
            }
        }
    }

    fn invoke_table(
        &mut self,
        function: TableFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        let table = args
            .table(0)
            .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
        let len = self
            .heap
            .table_array_len(table)
            .map_err(|error| self.error(proto, pc, error.to_string()))?;
        match function {
            TableFunction::Concat => {
                let separator = match args.get(1) {
                    None | Some(Value::Nil) => Vec::new(),
                    Some(_) => args
                        .bytes(1)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let start = args
                    .optional_integer(2, 1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let end = args
                    .optional_integer(3, len as i64)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let mut output = Vec::new();
                if start <= end {
                    for index in start..=end {
                        if index != start {
                            output.extend_from_slice(&separator);
                        }
                        let value = self
                            .heap
                            .table_get(table, &Value::Number(index as f64))
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                        match value {
                            Value::String(bytes) => output.extend_from_slice(bytes.as_bytes()),
                            Value::Number(number) => {
                                output.extend_from_slice(number.to_string().as_bytes())
                            }
                            other => {
                                return Err(self.error_kind(
                                    proto,
                                    pc,
                                    VmErrorKind::Type,
                                    format!(
                                        "invalid value ({}) at index {index} in table for 'concat'",
                                        other.type_name()
                                    ),
                                ));
                            }
                        }
                    }
                }
                Ok(vec![Value::String(ByteString::new(output))])
            }
            TableFunction::Insert => {
                let (position, value) = match args.len() {
                    2 => (len + 1, args.required(1).expect("length checked").clone()),
                    3 => {
                        let position = args
                            .integer(1)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                        if position < 1 || position > len as i64 + 1 {
                            return Err(self.error_kind(
                                proto,
                                pc,
                                VmErrorKind::Type,
                                "bad argument #2 to 'table.insert' (position out of bounds)",
                            ));
                        }
                        (
                            position as usize,
                            args.required(2).expect("length checked").clone(),
                        )
                    }
                    _ => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "wrong number of arguments to 'table.insert'",
                        ));
                    }
                };
                for index in (position..=len).rev() {
                    let item = self
                        .heap
                        .table_get(table, &Value::Number(index as f64))
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.heap
                        .table_set(table, Value::Number((index + 1) as f64), item)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                self.heap
                    .table_set(table, Value::Number(position as f64), value)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(Vec::new())
            }
            TableFunction::Remove => {
                if len == 0 {
                    return Ok(vec![Value::Nil]);
                }
                let position = args
                    .optional_integer(1, len as i64)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                if position < 1 || position > len as i64 {
                    return Ok(vec![Value::Nil]);
                }
                let position = position as usize;
                let removed = self
                    .heap
                    .table_get(table, &Value::Number(position as f64))
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                for index in position..len {
                    let item = self
                        .heap
                        .table_get(table, &Value::Number((index + 1) as f64))
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.heap
                        .table_set(table, Value::Number(index as f64), item)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                self.heap
                    .table_set(table, Value::Number(len as f64), Value::Nil)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![removed])
            }
            TableFunction::MaxN => {
                let mut maximum = 0.0_f64;
                let mut after = Value::Nil;
                while let Some((key, _)) = self
                    .heap
                    .table_next_pair(table, &after)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?
                {
                    if let Value::Number(number) = key {
                        maximum = maximum.max(number);
                        after = Value::Number(number);
                    } else {
                        after = key;
                    }
                }
                Ok(vec![Value::Number(maximum)])
            }
            TableFunction::Sort => {
                let comparator = args.get(1).filter(|value| **value != Value::Nil).cloned();
                let mut values = Vec::with_capacity(len);
                for index in 1..=len {
                    values.push(
                        self.heap
                            .table_get(table, &Value::Number(index as f64))
                            .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    );
                }
                for right in 1..values.len() {
                    let mut cursor = right;
                    while cursor > 0 {
                        let less = if let Some(comparator) = &comparator {
                            self.invoke_yield_forbidden(
                                comparator.clone(),
                                vec![values[cursor].clone(), values[cursor - 1].clone()],
                                proto,
                                pc,
                            )?
                            .first()
                            .is_some_and(Value::is_truthy)
                        } else {
                            compare(&values[cursor], &values[cursor - 1]).ok_or_else(|| {
                                self.error_kind(
                                    proto,
                                    pc,
                                    VmErrorKind::Type,
                                    "invalid order function for sorting",
                                )
                            })? == Ordering::Less
                        };
                        if !less {
                            break;
                        }
                        values.swap(cursor, cursor - 1);
                        cursor -= 1;
                    }
                }
                for (index, value) in values.into_iter().enumerate() {
                    self.heap
                        .table_set(table, Value::Number((index + 1) as f64), value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                Ok(Vec::new())
            }
        }
    }

    // ANCHOR: host_file_resources
    fn invoke_io(
        &mut self,
        function: IoFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        match function {
            IoFunction::Open => {
                let path = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let mode = match args.get(1) {
                    None | Some(Value::Nil) => b"r".to_vec(),
                    Some(_) => args
                        .bytes(1)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let path = match Self::host_path(&path) {
                    Ok(path) => path,
                    Err(message) => {
                        return Ok(vec![
                            Value::Nil,
                            Value::String(ByteString::new(message.as_bytes())),
                        ]);
                    }
                };
                let mut options = OpenOptions::new();
                let normalized = mode
                    .iter()
                    .copied()
                    .filter(|byte| *byte != b'b')
                    .collect::<Vec<_>>();
                match normalized.as_slice() {
                    b"r" => {
                        options.read(true);
                    }
                    b"w" => {
                        options.write(true).create(true).truncate(true);
                    }
                    b"a" => {
                        options.write(true).create(true).append(true);
                    }
                    b"r+" => {
                        options.read(true).write(true);
                    }
                    b"w+" => {
                        options.read(true).write(true).create(true).truncate(true);
                    }
                    b"a+" => {
                        options.read(true).write(true).create(true).append(true);
                    }
                    _ => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "bad argument #2 to 'io.open' (invalid mode)",
                        ));
                    }
                }
                match options.open(path) {
                    Ok(file) => {
                        let userdata = self
                            .heap
                            .allocate_userdata()
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                        self.heap
                            .userdata_set_metatable(userdata, Some(self.file_metatable))
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                        self.files.insert(userdata, Some(file));
                        Ok(vec![Value::Userdata(userdata)])
                    }
                    Err(error) => Ok(vec![
                        Value::Nil,
                        Value::String(ByteString::new(error.to_string().as_bytes())),
                    ]),
                }
            }
            IoFunction::Close => {
                let file = Self::native_userdata(args.get(0), function.name())
                    .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
                self.close_file(file, proto, pc)
            }
            IoFunction::Type => Ok(vec![match args.get(0) {
                Some(Value::Userdata(userdata)) if self.files.contains_key(userdata) => {
                    Value::String(ByteString::new(
                        if self.files.get(userdata).is_some_and(Option::is_some) {
                            &b"file"[..]
                        } else {
                            &b"closed file"[..]
                        },
                    ))
                }
                _ => Value::Nil,
            }]),
            IoFunction::Read => {
                let mut stdin = std::io::stdin().lock();
                let formats = if arguments.is_empty() {
                    vec![Value::String(ByteString::new(&b"*l"[..]))]
                } else {
                    arguments.to_vec()
                };
                Self::read_stream(&mut stdin, &formats)
                    .map_err(|message| self.error(proto, pc, message))
            }
            IoFunction::Write => {
                let mut output = Vec::new();
                for index in 0..args.len() {
                    output.extend(
                        args.bytes(index)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                    );
                }
                std::io::stdout()
                    .write_all(&output)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Boolean(true)])
            }
            IoFunction::Flush => {
                std::io::stdout()
                    .flush()
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Boolean(true)])
            }
        }
    }

    fn invoke_file(
        &mut self,
        function: FileFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        let handle = Self::native_userdata(args.get(0), function.name())
            .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Type, message))?;
        if !self.files.contains_key(&handle) {
            return Err(self.error_kind(proto, pc, VmErrorKind::Type, "not a file handle"));
        }
        if function != FileFunction::Close && self.files.get(&handle).is_some_and(Option::is_none) {
            return Err(self.error_kind(
                proto,
                pc,
                VmErrorKind::Runtime,
                "attempt to use a closed file",
            ));
        }
        match function {
            FileFunction::Close => self.close_file(handle, proto, pc),
            FileFunction::Read => {
                let formats = if arguments.len() <= 1 {
                    vec![Value::String(ByteString::new(&b"*l"[..]))]
                } else {
                    arguments[1..].to_vec()
                };
                let result = {
                    let file = self
                        .files
                        .get_mut(&handle)
                        .and_then(Option::as_mut)
                        .expect("open state checked above");
                    Self::read_file(file, &formats)
                };
                result.map_err(|message| self.error(proto, pc, message))
            }
            FileFunction::Write => {
                let mut output = Vec::new();
                for index in 1..args.len() {
                    output.extend(
                        args.bytes(index)
                            .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                    );
                }
                let result = self
                    .files
                    .get_mut(&handle)
                    .and_then(Option::as_mut)
                    .expect("open state checked above")
                    .write_all(&output);
                result.map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Userdata(handle)])
            }
            FileFunction::Flush => {
                let result = self
                    .files
                    .get_mut(&handle)
                    .and_then(Option::as_mut)
                    .expect("open state checked above")
                    .flush();
                result.map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Boolean(true)])
            }
            FileFunction::Seek => {
                let whence = match args.get(1) {
                    None | Some(Value::Nil) => b"cur".to_vec(),
                    Some(_) => args
                        .bytes(1)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let offset = args
                    .optional_integer(2, 0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let position = match whence.as_slice() {
                    b"set" if offset >= 0 => SeekFrom::Start(offset as u64),
                    b"cur" => SeekFrom::Current(offset),
                    b"end" => SeekFrom::End(offset),
                    _ => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "bad argument #2 to 'file:seek' (invalid option or offset)",
                        ));
                    }
                };
                let result = self
                    .files
                    .get_mut(&handle)
                    .and_then(Option::as_mut)
                    .expect("open state checked above")
                    .seek(position);
                let position = result.map_err(|error| self.error(proto, pc, error.to_string()))?;
                Ok(vec![Value::Number(position as f64)])
            }
        }
    }

    fn close_file(
        &mut self,
        handle: UserdataRef,
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        if !self.files.contains_key(&handle) {
            return Err(self.error_kind(proto, pc, VmErrorKind::Type, "not a file handle"));
        }
        let file = self.files.get_mut(&handle).expect("presence checked above");
        if file.take().is_none() {
            return Err(self.error_kind(
                proto,
                pc,
                VmErrorKind::Runtime,
                "attempt to close a closed file",
            ));
        }
        Ok(vec![Value::Boolean(true)])
    }

    fn read_file(file: &mut File, formats: &[Value]) -> Result<Vec<Value>, String> {
        let mut results = Vec::new();
        for format in formats {
            let value = match format {
                Value::String(format) if format.as_bytes() == b"*a" => {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    Value::String(ByteString::new(bytes))
                }
                Value::String(format) if format.as_bytes() == b"*l" => Self::read_line(file)?
                    .map_or(Value::Nil, |bytes| Value::String(ByteString::new(bytes))),
                Value::String(format) if format.as_bytes() == b"*n" => {
                    Self::read_number(file)?.map_or(Value::Nil, Value::Number)
                }
                Value::Number(count) if count.is_finite() && *count >= 0.0 => {
                    let mut bytes = vec![0; count.trunc() as usize];
                    let read = file.read(&mut bytes).map_err(|error| error.to_string())?;
                    bytes.truncate(read);
                    if read == 0 && *count > 0.0 {
                        Value::Nil
                    } else {
                        Value::String(ByteString::new(bytes))
                    }
                }
                other => return Err(format!("invalid read format ({})", other.type_name())),
            };
            let ended = value == Value::Nil;
            results.push(value);
            if ended {
                break;
            }
        }
        Ok(results)
    }

    fn read_stream(reader: &mut impl Read, formats: &[Value]) -> Result<Vec<Value>, String> {
        let mut results = Vec::new();
        for format in formats {
            let value = match format {
                Value::String(format) if format.as_bytes() == b"*a" => {
                    let mut bytes = Vec::new();
                    reader
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    Value::String(ByteString::new(bytes))
                }
                Value::String(format) if format.as_bytes() == b"*l" => Self::read_line(reader)?
                    .map_or(Value::Nil, |bytes| Value::String(ByteString::new(bytes))),
                Value::Number(count) if count.is_finite() && *count >= 0.0 => {
                    let mut bytes = vec![0; count.trunc() as usize];
                    let read = reader.read(&mut bytes).map_err(|error| error.to_string())?;
                    bytes.truncate(read);
                    if read == 0 && *count > 0.0 {
                        Value::Nil
                    } else {
                        Value::String(ByteString::new(bytes))
                    }
                }
                _ => return Err("stdin reader supports '*l', '*a', or a byte count".to_owned()),
            };
            let ended = value == Value::Nil;
            results.push(value);
            if ended {
                break;
            }
        }
        Ok(results)
    }

    fn read_line(reader: &mut impl Read) -> Result<Option<Vec<u8>>, String> {
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            match reader.read(&mut byte).map_err(|error| error.to_string())? {
                0 if line.is_empty() => return Ok(None),
                0 => break,
                _ if byte[0] == b'\n' => break,
                _ => line.push(byte[0]),
            }
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Ok(Some(line))
    }

    fn read_number(file: &mut File) -> Result<Option<f64>, String> {
        let start = file.stream_position().map_err(|error| error.to_string())?;
        let mut remaining = Vec::new();
        file.read_to_end(&mut remaining)
            .map_err(|error| error.to_string())?;
        let whitespace = remaining
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let token_len = remaining[whitespace..]
            .iter()
            .take_while(|byte| matches!(byte, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E'))
            .count();
        let token = &remaining[whitespace..whitespace + token_len];
        let number = std::str::from_utf8(token)
            .ok()
            .and_then(|text| text.parse::<f64>().ok());
        let consumed = if number.is_some() {
            whitespace + token_len
        } else {
            0
        };
        file.seek(SeekFrom::Start(start + consumed as u64))
            .map_err(|error| error.to_string())?;
        Ok(number)
    }
    // ANCHOR_END: host_file_resources

    fn invoke_os(
        &mut self,
        function: OsFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        match function {
            OsFunction::Clock => Ok(vec![Value::Number(self.start_time.elapsed().as_secs_f64())]),
            OsFunction::Time => {
                if !args.is_empty() && !matches!(args.get(0), Some(Value::Nil)) {
                    return Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "os.time table conversion is not supported in this checkpoint",
                    ));
                }
                Ok(vec![Value::Number(Self::unix_time() as f64)])
            }
            OsFunction::DiffTime => {
                let end = args
                    .number(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let start = args
                    .number(1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                Ok(vec![Value::Number(end - start)])
            }
            OsFunction::GetEnv => {
                let name = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let name = String::from_utf8(name).map_err(|_| {
                    self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "bad argument #1 to 'os.getenv' (UTF-8 variable name required)",
                    )
                })?;
                Ok(vec![std::env::var_os(name)
                    .map(|value| Value::String(ByteString::new(value.to_string_lossy().as_bytes())))
                    .unwrap_or(Value::Nil)])
            }
            OsFunction::Remove => {
                let path = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let result = Self::host_path(&path)
                    .and_then(|path| std::fs::remove_file(path).map_err(|error| error.to_string()));
                Ok(Self::os_result(result))
            }
            OsFunction::Rename => {
                let from = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let to = args
                    .bytes(1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let result = Self::host_path(&from).and_then(|from| {
                    Self::host_path(&to)
                        .and_then(|to| std::fs::rename(from, to).map_err(|error| error.to_string()))
                });
                Ok(Self::os_result(result))
            }
            OsFunction::TmpName => {
                self.temporary_name_counter = self.temporary_name_counter.wrapping_add(1);
                let path = std::env::temp_dir().join(format!(
                    "lua_rust_{}_{}.tmp",
                    std::process::id(),
                    self.temporary_name_counter
                ));
                Ok(vec![Value::String(ByteString::new(
                    path.to_string_lossy().as_bytes(),
                ))])
            }
            OsFunction::Execute => {
                let command = match args.get(0) {
                    None | Some(Value::Nil) => {
                        return Ok(vec![Value::Number(if cfg!(windows) { 1.0 } else { 0.0 })]);
                    }
                    Some(_) => args
                        .bytes(0)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let command = String::from_utf8(command).map_err(|_| {
                    self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        "bad argument #1 to 'os.execute' (UTF-8 command required)",
                    )
                })?;
                let status = if cfg!(windows) {
                    Command::new("cmd").args(["/C", &command]).status()
                } else {
                    Command::new("sh").args(["-c", &command]).status()
                };
                match status {
                    Ok(status) => Ok(vec![Value::Number(
                        status.code().map_or(1, |code| code) as f64
                    )]),
                    Err(error) => Ok(vec![
                        Value::Nil,
                        Value::String(ByteString::new(error.to_string().as_bytes())),
                    ]),
                }
            }
            OsFunction::Date => {
                let format = match args.get(0) {
                    None | Some(Value::Nil) => b"%c".to_vec(),
                    Some(_) => args
                        .bytes(0)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let timestamp = match args.get(1) {
                    None | Some(Value::Nil) => Self::unix_time(),
                    Some(_) => args
                        .integer(1)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?,
                };
                let date = Self::utc_date(timestamp);
                if format.as_slice() == b"*t" || format.as_slice() == b"!*t" {
                    let table = self
                        .heap
                        .allocate_table()
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    for (name, value) in [
                        (b"year".as_slice(), Value::Number(date.year as f64)),
                        (b"month", Value::Number(date.month as f64)),
                        (b"day", Value::Number(date.day as f64)),
                        (b"hour", Value::Number(date.hour as f64)),
                        (b"min", Value::Number(date.minute as f64)),
                        (b"sec", Value::Number(date.second as f64)),
                        (b"wday", Value::Number(date.weekday as f64)),
                        (b"yday", Value::Number(date.yearday as f64)),
                        (b"isdst", Value::Boolean(false)),
                    ] {
                        self.heap
                            .table_set(table, Value::String(ByteString::new(name)), value)
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    }
                    Ok(vec![Value::Table(table)])
                } else {
                    let format = format.strip_prefix(b"!").unwrap_or(&format);
                    Ok(vec![Value::String(ByteString::new(Self::format_utc_date(
                        format, date,
                    )))])
                }
            }
        }
    }

    // ANCHOR: lua_module_loader
    fn invoke_package(
        &mut self,
        function: PackageFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        match function {
            PackageFunction::Require => {
                let args = Args::new(arguments);
                let name = args
                    .bytes(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let key = Value::String(ByteString::new(&name[..]));
                let loaded = self
                    .heap
                    .table_get(self.package_loaded, &key)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                if loaded.is_truthy() {
                    return Ok(vec![loaded]);
                }

                let preload = self
                    .heap
                    .table_get(self.package_preload, &key)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                let loader = if preload != Value::Nil {
                    preload
                } else {
                    let package = self
                        .heap
                        .table_get(
                            self.global_env,
                            &Value::String(ByteString::new(&b"package"[..])),
                        )
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    let Value::Table(package) = package else {
                        return Err(self.error(proto, pc, "package table was replaced"));
                    };
                    let path = self
                        .heap
                        .table_get(package, &Value::String(ByteString::new(&b"path"[..])))
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    let Value::String(path) = path else {
                        return Err(self.error(proto, pc, "package.path must be a string"));
                    };
                    let module = String::from_utf8(name.clone()).map_err(|_| {
                        self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            "module name must be UTF-8 on this host",
                        )
                    })?;
                    let module_path = module.replace('.', "/");
                    let path_text = String::from_utf8(path.as_bytes().to_vec()).map_err(|_| {
                        self.error(proto, pc, "package.path must be UTF-8 on this host")
                    })?;
                    let mut diagnostics = Vec::new();
                    let mut found = None;
                    for template in path_text.split(';').filter(|entry| !entry.is_empty()) {
                        let candidate = template.replace('?', &module_path);
                        match std::fs::read(&candidate) {
                            Ok(source) => {
                                found = Some(self.compile_source(&source, proto, pc)?);
                                break;
                            }
                            Err(error) => {
                                diagnostics.push(format!("\n\tno file '{candidate}': {error}"))
                            }
                        }
                    }
                    let Some(function) = found else {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Runtime,
                            format!(
                                "module '{}' not found: no field package.preload['{}']{}",
                                module,
                                module,
                                diagnostics.concat()
                            ),
                        ));
                    };
                    Value::Function(function)
                };

                self.heap
                    .table_set(self.package_loaded, key.clone(), Value::Boolean(true))
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                let result = self.invoke_yield_forbidden(
                    loader,
                    vec![Value::String(ByteString::new(&name[..]))],
                    proto,
                    pc,
                );
                let results = match result {
                    Ok(results) => results,
                    Err(error) => {
                        self.heap
                            .table_set(self.package_loaded, key, Value::Nil)
                            .map_err(|cleanup| self.error(proto, pc, cleanup.to_string()))?;
                        return Err(error);
                    }
                };
                let returned = results.into_iter().next().unwrap_or(Value::Nil);
                if returned != Value::Nil {
                    self.heap
                        .table_set(self.package_loaded, key.clone(), returned.clone())
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    Ok(vec![returned])
                } else {
                    let loaded = self
                        .heap
                        .table_get(self.package_loaded, &key)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    Ok(vec![if loaded == Value::Nil {
                        Value::Boolean(true)
                    } else {
                        loaded
                    }])
                }
            }
        }
    }
    // ANCHOR_END: lua_module_loader

    fn invoke_debug(
        &mut self,
        function: DebugFunction,
        arguments: &[Value],
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let args = Args::new(arguments);
        match function {
            DebugFunction::GetRegistry => Ok(vec![Value::Table(self.registry)]),
            DebugFunction::GetMetatable => {
                let value = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let metatable = match value {
                    Value::Table(table) => self
                        .heap
                        .table_metatable(*table)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    Value::Userdata(userdata) => self
                        .heap
                        .userdata_metatable(*userdata)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    other => self.type_metatables.get(other.type_name()).copied(),
                };
                Ok(vec![metatable.map(Value::Table).unwrap_or(Value::Nil)])
            }
            DebugFunction::SetMetatable => {
                let value = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                    .clone();
                let metatable = match args.get(1) {
                    None | Some(Value::Nil) => None,
                    Some(Value::Table(table)) => Some(*table),
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!("bad argument #2 to 'debug.setmetatable' (table or nil expected, got {})", other.type_name()),
                        ));
                    }
                };
                match value {
                    Value::Table(table) => self
                        .heap
                        .table_set_metatable(table, metatable)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    Value::Userdata(userdata) => self
                        .heap
                        .userdata_set_metatable(userdata, metatable)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?,
                    other => {
                        if let Some(metatable) = metatable {
                            self.type_metatables.insert(other.type_name(), metatable);
                        } else {
                            self.type_metatables.remove(other.type_name());
                        }
                    }
                }
                Ok(vec![args.get(0).cloned().unwrap_or(Value::Nil)])
            }
            DebugFunction::GetUpvalue | DebugFunction::SetUpvalue => {
                let closure = match args.get(0) {
                    Some(Value::Function(function)) => *function,
                    Some(other) => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "bad argument #1 to '{}' (function expected, got {})",
                                function.name(),
                                other.type_name()
                            ),
                        ));
                    }
                    None => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "bad argument #1 to '{}' (function expected)",
                                function.name()
                            ),
                        ));
                    }
                };
                let index = args
                    .integer(1)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                if index < 1 {
                    return Ok(Vec::new());
                }
                let proto_owner = self
                    .heap
                    .function_proto(closure)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                let upvalues = self
                    .heap
                    .function_upvalues(closure)
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                let Some(upvalue) = upvalues.get(index as usize - 1).copied() else {
                    return Ok(Vec::new());
                };
                let name = proto_owner
                    .upvalues
                    .get(index as usize - 1)
                    .map(|upvalue| upvalue.name.clone())
                    .unwrap_or_else(|| ByteString::new(&b""[..]));
                if function == DebugFunction::GetUpvalue {
                    let value = self
                        .heap
                        .upvalue_get(upvalue)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    Ok(vec![Value::String(name), value])
                } else {
                    let value = args
                        .required(2)
                        .map_err(|error| self.host_error(function.name(), error, proto, pc))?
                        .clone();
                    self.heap
                        .upvalue_set(upvalue, value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    Ok(vec![Value::String(name)])
                }
            }
            DebugFunction::GetInfo => {
                let target = args
                    .required(0)
                    .map_err(|error| self.host_error(function.name(), error, proto, pc))?;
                let (what, source, nups, currentline) = match target {
                    Value::Function(function) => {
                        let owner = self
                            .heap
                            .function_proto(*function)
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                        (
                            b"Lua".as_slice(),
                            owner.debug_name.clone(),
                            owner.upvalues.len(),
                            owner.lines.first().copied().unwrap_or(0),
                        )
                    }
                    Value::NativeFunction(function) => {
                        (b"C".as_slice(), function.name().to_owned(), 0, 0)
                    }
                    Value::Number(level) if *level >= 0.0 => {
                        let index = level.trunc() as usize;
                        let Some(function) = self.active_functions.iter().rev().nth(index).copied()
                        else {
                            return Ok(vec![Value::Nil]);
                        };
                        let owner = self
                            .heap
                            .function_proto(function)
                            .map_err(|error| self.error(proto, pc, error.to_string()))?;
                        (
                            b"Lua".as_slice(),
                            owner.debug_name.clone(),
                            owner.upvalues.len(),
                            owner.lines.first().copied().unwrap_or(0),
                        )
                    }
                    other => {
                        return Err(self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!("bad argument #1 to 'debug.getinfo' (function or level expected, got {})", other.type_name()),
                        ));
                    }
                };
                let table = self
                    .heap
                    .allocate_table()
                    .map_err(|error| self.error(proto, pc, error.to_string()))?;
                for (name, value) in [
                    (b"what".as_slice(), Value::String(ByteString::new(what))),
                    (
                        b"source".as_slice(),
                        Value::String(ByteString::new(source.as_bytes())),
                    ),
                    (b"nups".as_slice(), Value::Number(nups as f64)),
                    (b"currentline".as_slice(), Value::Number(currentline as f64)),
                ] {
                    self.heap
                        .table_set(table, Value::String(ByteString::new(name)), value)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                }
                Ok(vec![Value::Table(table)])
            }
            DebugFunction::Traceback => {
                let message = args
                    .get(0)
                    .filter(|value| **value != Value::Nil)
                    .map(Self::stringify)
                    .unwrap_or_default();
                let mut output = message;
                if !output.is_empty() {
                    output.push(b'\n');
                }
                output.extend_from_slice(b"stack traceback:");
                for frame in self.call_stack.iter().rev() {
                    output.extend_from_slice(
                        format!("\n\t{}:{}", frame.source, frame.line).as_bytes(),
                    );
                }
                Ok(vec![Value::String(ByteString::new(output))])
            }
        }
    }

    fn compile_source(
        &mut self,
        source: &[u8],
        proto: &Proto,
        pc: usize,
    ) -> Result<FunctionRef, VmError> {
        let compiled = compile(source)
            .map_err(|error| self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string()))?;
        self.heap
            .allocate_function(Rc::new(compiled))
            .map_err(|error| self.error(proto, pc, error.to_string()))
    }

    fn load_file_function(
        &mut self,
        path: &[u8],
        proto: &Proto,
        pc: usize,
    ) -> Result<FunctionRef, VmError> {
        let path = Self::host_path(path)
            .map_err(|message| self.error_kind(proto, pc, VmErrorKind::Runtime, message))?;
        let source = std::fs::read(path)
            .map_err(|error| self.error_kind(proto, pc, VmErrorKind::Runtime, error.to_string()))?;
        self.compile_source(&source, proto, pc)
    }

    fn host_path(bytes: &[u8]) -> Result<PathBuf, String> {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| "host paths must be UTF-8 in this checkpoint".to_owned())
    }

    fn os_result(result: Result<(), String>) -> Vec<Value> {
        match result {
            Ok(()) => vec![Value::Boolean(true)],
            Err(message) => vec![
                Value::Nil,
                Value::String(ByteString::new(message.as_bytes())),
            ],
        }
    }

    fn unix_time() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as i64)
    }

    fn utc_date(timestamp: i64) -> DateParts {
        let days = timestamp.div_euclid(86_400);
        let seconds = timestamp.rem_euclid(86_400);
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = month_prime + if month_prime < 10 { 3 } else { -9 };
        year += i64::from(month <= 2);
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
            [(month - 1) as usize]
            + i64::from(leap && month > 2);
        DateParts {
            year,
            month,
            day,
            hour: seconds / 3_600,
            minute: seconds % 3_600 / 60,
            second: seconds % 60,
            weekday: (days + 4).rem_euclid(7) + 1,
            yearday: before_month + day,
        }
    }

    fn format_utc_date(format: &[u8], date: DateParts) -> Vec<u8> {
        let mut output = Vec::new();
        let mut cursor = 0;
        while cursor < format.len() {
            if format[cursor] != b'%' || cursor + 1 >= format.len() {
                output.push(format[cursor]);
                cursor += 1;
                continue;
            }
            let replacement = match format[cursor + 1] {
                b'Y' => format!("{:04}", date.year),
                b'm' => format!("{:02}", date.month),
                b'd' => format!("{:02}", date.day),
                b'H' => format!("{:02}", date.hour),
                b'M' => format!("{:02}", date.minute),
                b'S' => format!("{:02}", date.second),
                b'c' => format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    date.year, date.month, date.day, date.hour, date.minute, date.second
                ),
                b'%' => "%".to_owned(),
                other => format!("%{}", other as char),
            };
            output.extend_from_slice(replacement.as_bytes());
            cursor += 2;
        }
        output
    }
    // ANCHOR_END: stdlib_adapter

    fn host_error(&self, function: &str, error: HostError, proto: &Proto, pc: usize) -> VmError {
        self.error_kind(
            proto,
            pc,
            VmErrorKind::Type,
            format!("{error} to '{function}'"),
        )
    }

    fn string_index(index: i64, len: usize) -> i64 {
        if index < 0 {
            len as i64 + index + 1
        } else {
            index
        }
    }

    fn parse_integer_base(bytes: &[u8], base: u32) -> Option<f64> {
        let text = std::str::from_utf8(bytes).ok()?.trim();
        let (negative, digits) = text
            .strip_prefix('-')
            .map_or((false, text), |digits| (true, digits));
        let digits = digits.strip_prefix('+').unwrap_or(digits);
        let integer = i128::from_str_radix(digits, base).ok()?;
        Some(if negative {
            -(integer as f64)
        } else {
            integer as f64
        })
    }

    fn native_table(value: Option<&Value>, name: &str) -> Result<TableRef, String> {
        match value {
            Some(Value::Table(table)) => Ok(*table),
            Some(other) => Err(format!("{name} expected table, got {}", other.type_name())),
            None => Err(format!("{name} expected table, got no value")),
        }
    }

    fn native_userdata(value: Option<&Value>, name: &str) -> Result<UserdataRef, String> {
        match value {
            Some(Value::Userdata(userdata)) => Ok(*userdata),
            Some(other) => Err(format!(
                "{name} expected userdata, got {}",
                other.type_name()
            )),
            None => Err(format!("{name} expected userdata, got no value")),
        }
    }

    fn native_coroutine(value: Option<&Value>, name: &str) -> Result<CoroutineRef, String> {
        match value {
            Some(Value::Coroutine(coroutine)) => Ok(*coroutine),
            Some(other) => Err(format!("{name} expected thread, got {}", other.type_name())),
            None => Err(format!("{name} expected thread, got no value")),
        }
    }

    fn supported_type_name(name: &[u8]) -> Option<&'static str> {
        match name {
            b"nil" => Some("nil"),
            b"boolean" => Some("boolean"),
            b"number" => Some("number"),
            b"string" => Some("string"),
            b"function" => Some("function"),
            b"userdata" => Some("userdata"),
            b"thread" => Some("thread"),
            _ => None,
        }
    }

    fn stringify(value: &Value) -> Vec<u8> {
        match value {
            Value::Nil => b"nil".to_vec(),
            Value::Boolean(value) => value.to_string().into_bytes(),
            Value::Number(value) => value.to_string().into_bytes(),
            Value::String(value) => value.as_bytes().to_vec(),
            Value::Table(value) => format!("table: 0x{:x}", value.identity()).into_bytes(),
            Value::Function(value) => format!("function: 0x{:x}", value.identity()).into_bytes(),
            Value::Userdata(value) => format!("userdata: 0x{:x}", value.identity()).into_bytes(),
            Value::Coroutine(value) => format!("thread: 0x{:x}", value.identity()).into_bytes(),
            Value::CoroutineWrapper(value) => {
                format!("function: coroutine:0x{:x}", value.identity()).into_bytes()
            }
            Value::NativeFunction(value) => {
                format!("function: builtin:{}", value.name()).into_bytes()
            }
        }
    }

    // ANCHOR: open_upvalues
    fn capture_upvalue(
        &mut self,
        frame_id: u64,
        register: u8,
        registers: &[StackSlot],
        proto: &Proto,
        pc: usize,
    ) -> Result<UpvalueRef, VmError> {
        if let Some(upvalue) = self.open_upvalues.get(&(frame_id, register)).copied() {
            match self.heap.is_live(upvalue) {
                Ok(true) => return Ok(upvalue),
                Ok(false) => {
                    self.open_upvalues.remove(&(frame_id, register));
                }
                Err(error) => return Err(self.error(proto, pc, error.to_string())),
            }
        }
        let slot = registers
            .get(usize::from(register))
            .cloned()
            .ok_or_else(|| {
                self.error(
                    proto,
                    pc,
                    format!("captured register {register} is out of bounds"),
                )
            })?;
        let upvalue = self
            .heap
            .allocate_open_upvalue(slot)
            .map_err(|error| self.error(proto, pc, error.to_string()))?;
        self.open_upvalues.insert((frame_id, register), upvalue);
        Ok(upvalue)
    }

    fn close_upvalues(&mut self, frame_id: u64, first_register: u8) -> Result<(), HeapError> {
        let keys = self
            .open_upvalues
            .keys()
            .filter(|(owner, register)| *owner == frame_id && *register >= first_register)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(upvalue) = self.open_upvalues.remove(&key) {
                self.heap.close_upvalue(upvalue)?;
            }
        }
        Ok(())
    }
    // ANCHOR_END: open_upvalues

    // ANCHOR: metamethod_dispatch
    fn metatable_for(
        &self,
        value: &Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<Option<TableRef>, VmError> {
        if let Value::Table(table) = value {
            return self.heap.table_metatable(*table).map_err(|error| {
                self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
            });
        }
        if let Value::Userdata(userdata) = value {
            return self.heap.userdata_metatable(*userdata).map_err(|error| {
                self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
            });
        }
        Ok(self.type_metatables.get(value.type_name()).copied())
    }

    fn metamethod(
        &self,
        value: &Value,
        event: &'static [u8],
        proto: &Proto,
        pc: usize,
    ) -> Result<Option<Value>, VmError> {
        let Some(metatable) = self.metatable_for(value, proto, pc)? else {
            return Ok(None);
        };
        let method = self
            .heap
            .table_get(metatable, &Value::String(ByteString::new(event)))
            .map_err(|error| {
                self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
            })?;
        Ok((method != Value::Nil).then_some(method))
    }

    fn resolve_callable(
        &mut self,
        mut callable: Value,
        mut arguments: Vec<Value>,
        proto: &Proto,
        pc: usize,
    ) -> Result<(Value, Vec<Value>, bool), VmError> {
        let mut crossed_call_boundary = false;
        for _ in 0..METAMETHOD_CHAIN_LIMIT {
            if matches!(
                callable,
                Value::Function(_) | Value::NativeFunction(_) | Value::CoroutineWrapper(_)
            ) {
                return Ok((callable, arguments, crossed_call_boundary));
            }
            let Some(method) = self.metamethod(&callable, b"__call", proto, pc)? else {
                return Err(self.error_kind(
                    proto,
                    pc,
                    VmErrorKind::Type,
                    format!("attempt to call a {} value", callable.type_name()),
                ));
            };
            arguments.insert(0, callable);
            callable = method;
            crossed_call_boundary = true;
        }
        Err(self.error_kind(proto, pc, VmErrorKind::Limit, "__call chain limit exceeded"))
    }

    fn invoke_callable(
        &mut self,
        callable: Value,
        arguments: Vec<Value>,
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        let (callable, arguments, crossed_call_boundary) =
            self.resolve_callable(callable, arguments, proto, pc)?;
        if crossed_call_boundary {
            self.yield_forbidden_depth += 1;
        }
        let result = self.invoke(callable, &arguments, proto, pc, 0, arguments.len());
        if crossed_call_boundary {
            self.yield_forbidden_depth -= 1;
        }
        result
    }

    fn index_value(
        &mut self,
        mut object: Value,
        key: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<Value, VmError> {
        for _ in 0..METAMETHOD_CHAIN_LIMIT {
            if let Value::Table(table) = &object {
                let raw = self.heap.table_get(*table, &key).map_err(|error| {
                    self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                })?;
                if raw != Value::Nil {
                    return Ok(raw);
                }
            }
            let Some(method) = self.metamethod(&object, b"__index", proto, pc)? else {
                return if matches!(object, Value::Table(_)) {
                    Ok(Value::Nil)
                } else {
                    Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        format!("attempt to index a {} value", object.type_name()),
                    ))
                };
            };
            if matches!(
                method,
                Value::Function(_) | Value::NativeFunction(_) | Value::CoroutineWrapper(_)
            ) {
                return Ok(self
                    .invoke_yield_forbidden(method, vec![object, key], proto, pc)?
                    .into_iter()
                    .next()
                    .unwrap_or(Value::Nil));
            }
            object = method;
        }
        Err(self.error_kind(
            proto,
            pc,
            VmErrorKind::Limit,
            "__index chain limit exceeded",
        ))
    }

    fn set_indexed(
        &mut self,
        mut object: Value,
        key: Value,
        value: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<(), VmError> {
        for _ in 0..METAMETHOD_CHAIN_LIMIT {
            if let Value::Table(table) = &object {
                let raw = self.heap.table_get(*table, &key).map_err(|error| {
                    self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                })?;
                if raw != Value::Nil {
                    return self.heap.table_set(*table, key, value).map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                    });
                }
            }
            let Some(method) = self.metamethod(&object, b"__newindex", proto, pc)? else {
                return if let Value::Table(table) = object {
                    self.heap.table_set(table, key, value).map_err(|error| {
                        self.error_kind(proto, pc, VmErrorKind::Metamethod, error.to_string())
                    })
                } else {
                    Err(self.error_kind(
                        proto,
                        pc,
                        VmErrorKind::Type,
                        format!("attempt to index a {} value", object.type_name()),
                    ))
                };
            };
            if matches!(
                method,
                Value::Function(_) | Value::NativeFunction(_) | Value::CoroutineWrapper(_)
            ) {
                self.invoke_yield_forbidden(method, vec![object, key, value], proto, pc)?;
                return Ok(());
            }
            object = method;
        }
        Err(self.error_kind(
            proto,
            pc,
            VmErrorKind::Limit,
            "__newindex chain limit exceeded",
        ))
    }

    fn first_metamethod(
        &self,
        left: &Value,
        right: &Value,
        event: &'static [u8],
        proto: &Proto,
        pc: usize,
    ) -> Result<Option<Value>, VmError> {
        if let Some(method) = self.metamethod(left, event, proto, pc)? {
            return Ok(Some(method));
        }
        self.metamethod(right, event, proto, pc)
    }

    fn shared_metamethod(
        &self,
        left: &Value,
        right: &Value,
        event: &'static [u8],
        proto: &Proto,
        pc: usize,
    ) -> Result<Option<Value>, VmError> {
        let left_method = self.metamethod(left, event, proto, pc)?;
        let right_method = self.metamethod(right, event, proto, pc)?;
        Ok(match (left_method, right_method) {
            (Some(left), Some(right)) if left == right => Some(left),
            _ => None,
        })
    }

    fn call_event(
        &mut self,
        method: Value,
        arguments: Vec<Value>,
        proto: &Proto,
        pc: usize,
    ) -> Result<Value, VmError> {
        Ok(self
            .invoke_yield_forbidden(method, arguments, proto, pc)?
            .into_iter()
            .next()
            .unwrap_or(Value::Nil))
    }

    fn invoke_yield_forbidden(
        &mut self,
        callable: Value,
        arguments: Vec<Value>,
        proto: &Proto,
        pc: usize,
    ) -> Result<Vec<Value>, VmError> {
        self.yield_forbidden_depth += 1;
        let result = self.invoke_callable(callable, arguments, proto, pc);
        self.yield_forbidden_depth -= 1;
        result
    }
    // ANCHOR_END: metamethod_dispatch

    fn write_results(
        &self,
        registers: &mut Vec<StackSlot>,
        base: usize,
        results: &[Value],
        wanted: Option<usize>,
        proto: &Proto,
        pc: usize,
    ) -> Result<usize, VmError> {
        let count = wanted.unwrap_or(results.len());
        let end = base
            .checked_add(count)
            .ok_or_else(|| self.error(proto, pc, "result window arithmetic overflow"))?;
        if end > 255 {
            return Err(self.error(proto, pc, "result window exceeds 255 registers"));
        }
        if registers.len() < end {
            registers.resize_with(end, || Rc::new(RefCell::new(Value::Nil)));
        }
        for offset in 0..count {
            *registers[base + offset].borrow_mut() =
                results.get(offset).cloned().unwrap_or(Value::Nil);
        }
        Ok(end)
    }
    // ANCHOR_END: variable_results

    fn binary(
        &mut self,
        opcode: Opcode,
        left: Value,
        right: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<Value, VmError> {
        match opcode {
            Opcode::Eq | Opcode::Ne => {
                let equal = if left == right {
                    true
                } else if matches!((&left, &right), (Value::Table(_), Value::Table(_))) {
                    if let Some(method) =
                        self.shared_metamethod(&left, &right, b"__eq", proto, pc)?
                    {
                        self.call_event(method, vec![left.clone(), right.clone()], proto, pc)?
                            .is_truthy()
                    } else {
                        false
                    }
                } else {
                    false
                };
                Ok(Value::Boolean(if opcode == Opcode::Eq {
                    equal
                } else {
                    !equal
                }))
            }
            Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge => {
                if let Some(ordering) = compare(&left, &right) {
                    return Ok(Value::Boolean(match opcode {
                        Opcode::Lt => ordering == Ordering::Less,
                        Opcode::Le => ordering != Ordering::Greater,
                        Opcode::Gt => ordering == Ordering::Greater,
                        Opcode::Ge => ordering != Ordering::Less,
                        _ => unreachable!(),
                    }));
                }
                let result = match opcode {
                    Opcode::Lt | Opcode::Gt => {
                        let (first, second) = if opcode == Opcode::Gt {
                            (&right, &left)
                        } else {
                            (&left, &right)
                        };
                        let method = self
                            .shared_metamethod(first, second, b"__lt", proto, pc)?
                            .ok_or_else(|| {
                                self.error_kind(
                                    proto,
                                    pc,
                                    VmErrorKind::Type,
                                    format!(
                                        "cannot compare {} with {}",
                                        left.type_name(),
                                        right.type_name()
                                    ),
                                )
                            })?;
                        self.call_event(method, vec![first.clone(), second.clone()], proto, pc)?
                            .is_truthy()
                    }
                    Opcode::Le | Opcode::Ge => {
                        let (first, second) = if opcode == Opcode::Ge {
                            (&right, &left)
                        } else {
                            (&left, &right)
                        };
                        if let Some(method) =
                            self.shared_metamethod(first, second, b"__le", proto, pc)?
                        {
                            self.call_event(method, vec![first.clone(), second.clone()], proto, pc)?
                                .is_truthy()
                        } else {
                            let method = self
                                .shared_metamethod(second, first, b"__lt", proto, pc)?
                                .ok_or_else(|| {
                                    self.error_kind(
                                        proto,
                                        pc,
                                        VmErrorKind::Type,
                                        format!(
                                            "cannot compare {} with {}",
                                            left.type_name(),
                                            right.type_name()
                                        ),
                                    )
                                })?;
                            !self
                                .call_event(method, vec![second.clone(), first.clone()], proto, pc)?
                                .is_truthy()
                        }
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Boolean(result))
            }
            Opcode::Concat => {
                if let (Some(mut bytes), Some(right_bytes)) =
                    (concat_bytes(left.clone()), concat_bytes(right.clone()))
                {
                    bytes.extend(right_bytes);
                    return Ok(Value::String(ByteString::new(bytes)));
                }
                let method = self
                    .first_metamethod(&left, &right, b"__concat", proto, pc)?
                    .ok_or_else(|| {
                        self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "cannot concatenate {} with {}",
                                left.type_name(),
                                right.type_name()
                            ),
                        )
                    })?;
                self.call_event(method, vec![left, right], proto, pc)
            }
            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Mod | Opcode::Pow => {
                if let (Value::Number(left_number), Value::Number(right_number)) = (&left, &right) {
                    return Ok(Value::Number(match opcode {
                        Opcode::Add => left_number + right_number,
                        Opcode::Sub => left_number - right_number,
                        Opcode::Mul => left_number * right_number,
                        Opcode::Div => left_number / right_number,
                        Opcode::Mod => {
                            left_number - (left_number / right_number).floor() * right_number
                        }
                        Opcode::Pow => left_number.powf(*right_number),
                        _ => unreachable!(),
                    }));
                }
                let (event, event_name) = match opcode {
                    Opcode::Add => (b"__add".as_slice(), "__add"),
                    Opcode::Sub => (b"__sub".as_slice(), "__sub"),
                    Opcode::Mul => (b"__mul".as_slice(), "__mul"),
                    Opcode::Div => (b"__div".as_slice(), "__div"),
                    Opcode::Mod => (b"__mod".as_slice(), "__mod"),
                    Opcode::Pow => (b"__pow".as_slice(), "__pow"),
                    _ => unreachable!(),
                };
                let method = self
                    .first_metamethod(&left, &right, event, proto, pc)?
                    .ok_or_else(|| {
                        self.error_kind(
                            proto,
                            pc,
                            VmErrorKind::Type,
                            format!(
                                "arithmetic needs numbers or {event_name} metamethod, got {} and {}",
                                left.type_name(),
                                right.type_name()
                            ),
                        )
                    })?;
                self.call_event(method, vec![left, right], proto, pc)
            }
            _ => unreachable!("caller only passes eager binary opcodes"),
        }
    }

    fn read(
        &self,
        registers: &[StackSlot],
        register: u16,
        proto: &Proto,
        pc: usize,
    ) -> Result<Value, VmError> {
        registers
            .get(usize::from(register))
            .map(|slot| slot.borrow().clone())
            .ok_or_else(|| self.error(proto, pc, format!("register {register} is out of bounds")))
    }

    fn number_register(
        &self,
        registers: &[StackSlot],
        register: u8,
        proto: &Proto,
        pc: usize,
    ) -> Result<f64, VmError> {
        match self.read(registers, u16::from(register), proto, pc)? {
            Value::Number(value) => Ok(value),
            other => Err(self.error(
                proto,
                pc,
                format!("numeric for expected number, got {}", other.type_name()),
            )),
        }
    }

    fn table_register(
        &self,
        registers: &[StackSlot],
        register: u16,
        proto: &Proto,
        pc: usize,
    ) -> Result<TableRef, VmError> {
        match self.read(registers, register, proto, pc)? {
            Value::Table(value) => Ok(value),
            other => Err(self.error_kind(
                proto,
                pc,
                VmErrorKind::Type,
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
        registers: &[StackSlot],
        register: u8,
        value: Value,
        proto: &Proto,
        pc: usize,
    ) -> Result<(), VmError> {
        let slot = registers.get(usize::from(register)).ok_or_else(|| {
            self.error(proto, pc, format!("register {register} is out of bounds"))
        })?;
        *slot.borrow_mut() = value;
        Ok(())
    }

    fn error(&self, proto: &Proto, pc: usize, message: impl Into<String>) -> VmError {
        self.error_kind(proto, pc, VmErrorKind::Runtime, message)
    }

    fn error_kind(
        &self,
        proto: &Proto,
        pc: usize,
        kind: VmErrorKind,
        message: impl Into<String>,
    ) -> VmError {
        VmError {
            kind,
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

impl Drop for Vm {
    fn drop(&mut self) {
        self.files.clear();
        self.heap.shutdown();
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
        Value::Nil
        | Value::Boolean(_)
        | Value::Table(_)
        | Value::Function(_)
        | Value::Userdata(_)
        | Value::Coroutine(_)
        | Value::CoroutineWrapper(_)
        | Value::NativeFunction(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Vm;
    use lua_core::{ByteString, Instruction, Opcode, Proto, Value};

    #[test]
    fn malformed_register_is_rejected_instead_of_panicking() {
        let proto = Proto {
            constants: vec![],
            code: vec![Instruction::abc(Opcode::Return, 7, 2, 0)],
            lines: vec![12],
            max_stack_size: 1,
            protos: vec![],
            num_params: 0,
            is_vararg: false,
            upvalues: vec![],
            debug_name: "malformed-register".to_owned(),
        };
        let error = Vm::new().execute(&proto).unwrap_err();
        assert_eq!(error.line, 12);
        assert!(error.message.contains("return window is out of bounds"));
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
            is_vararg: false,
            upvalues: vec![],
            debug_name: "malformed-constant".to_owned(),
        };
        assert!(Vm::new()
            .execute(&proto)
            .unwrap_err()
            .message
            .contains("constant index"));
    }

    #[test]
    fn registry_keeps_host_values_alive_until_the_entry_is_removed() {
        let proto = Proto {
            constants: vec![],
            code: vec![
                Instruction::abc(Opcode::NewTable, 0, 0, 0),
                Instruction::abc(Opcode::Return, 0, 2, 0),
            ],
            lines: vec![1, 1],
            max_stack_size: 1,
            protos: vec![],
            num_params: 0,
            is_vararg: false,
            upvalues: vec![],
            debug_name: "registry-root".to_owned(),
        };
        let mut vm = Vm::new();
        let table = vm.execute(&proto).unwrap();
        let key = Value::String(ByteString::new(&b"host"[..]));
        vm.registry_set(key.clone(), table.clone()).unwrap();

        vm.collect_garbage().unwrap();
        assert_eq!(vm.registry_get(&key), Ok(table));

        vm.registry_set(key.clone(), Value::Nil).unwrap();
        assert!(vm.collect_garbage().unwrap().collected >= 1);
        assert_eq!(vm.registry_get(&key), Ok(Value::Nil));
    }
}

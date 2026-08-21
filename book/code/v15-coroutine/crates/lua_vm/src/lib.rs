use lua_core::{
    ByteString, CoroutineRef, FunctionRef, GcPhase as HeapPhase, GcStats, GcStep, Heap, HeapError,
    NativeFunction, Opcode, Proto, StackSlot, TableRef, UpvalueRef, UserdataRef, Value,
};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

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
    globals: HashMap<ByteString, Value>,
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
        let mut globals = HashMap::new();
        for (name, function) in [
            (b"next".as_slice(), NativeFunction::Next),
            (b"pairs".as_slice(), NativeFunction::Pairs),
            (b"ipairs".as_slice(), NativeFunction::IPairs),
            (b"setmetatable".as_slice(), NativeFunction::SetMetatable),
            (b"getmetatable".as_slice(), NativeFunction::GetMetatable),
            (b"rawget".as_slice(), NativeFunction::RawGet),
            (b"rawset".as_slice(), NativeFunction::RawSet),
            (b"tostring".as_slice(), NativeFunction::ToString),
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
            globals.insert(ByteString::new(name), Value::NativeFunction(function));
        }
        let coroutine_library = heap
            .allocate_table()
            .expect("a fresh teaching heap can allocate the coroutine library");
        for (name, function) in [
            (b"create".as_slice(), NativeFunction::CoroutineCreate),
            (b"resume".as_slice(), NativeFunction::CoroutineResume),
            (b"yield".as_slice(), NativeFunction::CoroutineYield),
            (b"status".as_slice(), NativeFunction::CoroutineStatus),
            (b"running".as_slice(), NativeFunction::CoroutineRunning),
            (b"wrap".as_slice(), NativeFunction::CoroutineWrap),
        ] {
            heap.table_set(
                coroutine_library,
                Value::String(ByteString::new(name)),
                Value::NativeFunction(function),
            )
            .expect("a fresh teaching heap can initialize the coroutine library");
        }
        globals.insert(
            ByteString::new(&b"coroutine"[..]),
            Value::Table(coroutine_library),
        );
        Self {
            instruction_limit: 1_000_000,
            remaining_instructions: 1_000_000,
            globals,
            max_call_depth: 32,
            call_stack: Vec::new(),
            next_frame_id: 1,
            open_upvalues: HashMap::new(),
            type_metatables: HashMap::new(),
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
        let mut roots = self.globals.values().cloned().collect::<Vec<_>>();
        roots.push(Value::Table(self.registry));
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
                    let value = self.globals.get(name).cloned().unwrap_or(Value::Nil);
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
                    if value == Value::Nil {
                        self.globals.remove(name);
                    } else {
                        self.globals.insert(name.clone(), value);
                    }
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

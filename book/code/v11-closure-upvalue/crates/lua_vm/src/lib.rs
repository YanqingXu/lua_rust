use lua_core::{
    ByteString, FunctionRef, NativeFunction, Opcode, Proto, StackSlot, TableRef, UpvalueRef, Value,
};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

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
    remaining_instructions: usize,
    globals: HashMap<ByteString, Value>,
    max_call_depth: usize,
    call_stack: Vec<CallInfo>,
    next_frame_id: u64,
    open_upvalues: HashMap<(u64, u8), UpvalueRef>,
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

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        for (name, function) in [
            (b"next".as_slice(), NativeFunction::Next),
            (b"pairs".as_slice(), NativeFunction::Pairs),
            (b"ipairs".as_slice(), NativeFunction::IPairs),
        ] {
            globals.insert(ByteString::new(name), Value::NativeFunction(function));
        }
        Self {
            instruction_limit: 1_000_000,
            remaining_instructions: 1_000_000,
            globals,
            max_call_depth: 32,
            call_stack: Vec::new(),
            next_frame_id: 1,
            open_upvalues: HashMap::new(),
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
        self.open_upvalues.clear();
        self.next_frame_id = 1;
        self.remaining_instructions = self.instruction_limit;
        self.execute_proto(FunctionRef::new(Rc::new(proto.clone())), &[])
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
        let result = self.run_frame(function, arguments, frame_id);
        self.close_upvalues(frame_id, 0);
        result
    }

    fn run_frame(
        &mut self,
        function: &FunctionRef,
        arguments: &[Value],
        frame_id: u64,
    ) -> Result<FrameOutcome, VmError> {
        let proto = function.proto();
        proto.validate_shape().map_err(|message| VmError {
            pc: 0,
            line: 0,
            message: message.to_owned(),
            traceback: Vec::new(),
        })?;
        let mut registers = (0..usize::from(proto.max_stack_size))
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
        let mut top = usize::from(proto.num_params).min(registers.len());
        let mut pc = 0_usize;
        while pc < proto.code.len() {
            if self.remaining_instructions == 0 {
                return Err(self.error(proto, pc, "instruction limit exceeded"));
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
                            function
                                .upvalues()
                                .get(usize::from(descriptor.index))
                                .cloned()
                                .ok_or_else(|| {
                                    self.error(proto, pc, "parent upvalue index is out of bounds")
                                })?
                        };
                        upvalues.push(upvalue);
                    }
                    self.write(
                        &registers,
                        instruction.a(),
                        Value::Function(FunctionRef::with_upvalues(nested, upvalues)),
                        proto,
                        pc,
                    )?;
                }
                Opcode::GetUpval => {
                    let upvalue = function
                        .upvalues()
                        .get(usize::from(instruction.b()))
                        .ok_or_else(|| self.error(proto, pc, "upvalue index is out of bounds"))?;
                    self.write(&registers, instruction.a(), upvalue.get(), proto, pc)?;
                }
                Opcode::SetUpval => {
                    let upvalue = function
                        .upvalues()
                        .get(usize::from(instruction.b()))
                        .ok_or_else(|| self.error(proto, pc, "upvalue index is out of bounds"))?;
                    let value = self.read(&registers, u16::from(instruction.a()), proto, pc)?;
                    upvalue.set(value);
                }
                Opcode::Close => {
                    self.close_upvalues(frame_id, instruction.a());
                }
                Opcode::Call | Opcode::TailCall => {
                    let function = self
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
                    let tail = opcode == Opcode::TailCall;
                    if tail {
                        match function {
                            Value::Function(function) => {
                                return Ok(FrameOutcome::TailCall(function, arguments));
                            }
                            Value::NativeFunction(function) => {
                                let results = self
                                    .invoke_native(function, &arguments)
                                    .map_err(|message| self.error(proto, pc, message))?;
                                return Ok(FrameOutcome::Return(results));
                            }
                            other => {
                                return Err(self.error(
                                    proto,
                                    pc,
                                    format!("attempt to call a {} value", other.type_name()),
                                ));
                            }
                        }
                    }
                    let results = self.invoke(function, &arguments, proto, pc, first, end)?;
                    let wanted = if instruction.c() == 0 {
                        None
                    } else {
                        Some(usize::from(instruction.c() - 1))
                    };
                    top = self.write_results(
                        &mut registers,
                        usize::from(instruction.a()),
                        &results,
                        wanted,
                        proto,
                        pc,
                    )?;
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
                }
                // ANCHOR: table_vm
                Opcode::NewTable => {
                    self.write(
                        &registers,
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
                        .get(&key)
                        .map_err(|error| self.error(proto, pc, error.to_string()))?;
                    self.write(&registers, instruction.a(), value, proto, pc)?;
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
                        table
                            .set(Value::Number((start + offset) as f64), value)
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
        match function {
            Value::Function(function) => {
                if self.call_stack.len() >= self.max_call_depth {
                    return Err(self.error(caller, pc, "call depth limit exceeded"));
                }
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
            Value::NativeFunction(function) => self
                .invoke_native(function, arguments)
                .map_err(|message| self.error(caller, pc, message)),
            other => Err(self.error(
                caller,
                pc,
                format!("attempt to call a {} value", other.type_name()),
            )),
        }
    }

    fn invoke_native(
        &self,
        function: NativeFunction,
        arguments: &[Value],
    ) -> Result<Vec<Value>, String> {
        match function {
            NativeFunction::Next => {
                let table = Self::native_table(arguments.first(), "next")?;
                let after = arguments.get(1).cloned().unwrap_or(Value::Nil);
                Ok(
                    match table.next_pair(&after).map_err(|error| error.to_string())? {
                        Some((key, value)) => vec![key, value],
                        None => vec![Value::Nil],
                    },
                )
            }
            NativeFunction::Pairs => {
                let table = Self::native_table(arguments.first(), "pairs")?;
                Ok(vec![
                    Value::NativeFunction(NativeFunction::Next),
                    Value::Table(table),
                    Value::Nil,
                ])
            }
            NativeFunction::IPairs => {
                let table = Self::native_table(arguments.first(), "ipairs")?;
                Ok(vec![
                    Value::NativeFunction(NativeFunction::IPairsAux),
                    Value::Table(table),
                    Value::Number(0.0),
                ])
            }
            NativeFunction::IPairsAux => {
                let table = Self::native_table(arguments.first(), "ipairs iterator")?;
                let index = match arguments.get(1) {
                    Some(Value::Number(index)) => *index,
                    Some(other) => {
                        return Err(format!(
                            "ipairs iterator index must be number, got {}",
                            other.type_name()
                        ));
                    }
                    None => 0.0,
                } + 1.0;
                let value = table
                    .get(&Value::Number(index))
                    .map_err(|error| error.to_string())?;
                if value == Value::Nil {
                    Ok(vec![Value::Nil])
                } else {
                    Ok(vec![Value::Number(index), value])
                }
            }
        }
    }

    fn native_table(value: Option<&Value>, name: &str) -> Result<TableRef, String> {
        match value {
            Some(Value::Table(table)) => Ok(table.clone()),
            Some(other) => Err(format!("{name} expected table, got {}", other.type_name())),
            None => Err(format!("{name} expected table, got no value")),
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
        if let Some(upvalue) = self.open_upvalues.get(&(frame_id, register)) {
            return Ok(upvalue.clone());
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
        let upvalue = UpvalueRef::open(slot);
        self.open_upvalues
            .insert((frame_id, register), upvalue.clone());
        Ok(upvalue)
    }

    fn close_upvalues(&mut self, frame_id: u64, first_register: u8) {
        let keys = self
            .open_upvalues
            .keys()
            .filter(|(owner, register)| *owner == frame_id && *register >= first_register)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(upvalue) = self.open_upvalues.remove(&key) {
                upvalue.close();
            }
        }
    }
    // ANCHOR_END: open_upvalues

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
        Value::Nil
        | Value::Boolean(_)
        | Value::Table(_)
        | Value::Function(_)
        | Value::NativeFunction(_) => None,
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
}

//! Runtime-scoped instruction trace support.
//!
//! Snapshot mode records the complete register window immediately before each
//! instruction. Diff mode records the post-instruction register changes. Both
//! include the fixed-C++ logical stack top without mutating Rust VM state. The
//! CALL/RETURN events mirror the fixed C++ opcode boundaries, and top-level
//! unhandled errors are recorded at the Runtime execution boundary. Upvalue
//! open/close events are deferred until the owning instruction evidence is
//! complete. Coroutine resume/yield and collected-state retirement events use
//! the same ordered stream and stable runtime state handles.

use std::cell::RefCell;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;

use lua_compiler::opcode;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::{Instruction, Proto};
use lua_core::state_handle::StateHandle;
use lua_core::value::Value;
use thiserror::Error;

use crate::state::LuaState;

pub(crate) type SharedInstructionTrace = Rc<RefCell<InstructionTrace>>;

/// Register evidence emitted for each traced VM instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstructionTraceMode {
    /// Emit the complete register window immediately before execution.
    #[default]
    Snapshot,
    /// Emit only registers changed by the completed instruction.
    Diff,
}

/// Summary returned after a trace stream is disabled and flushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionTraceSummary {
    pub event_count: u64,
}

/// A deferred write or flush failure from an instruction trace stream.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("instruction trace output failed: {message}")]
pub struct InstructionTraceError {
    message: String,
}

impl InstructionTraceError {
    fn from_io(error: io::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

pub(crate) struct InstructionTrace {
    output: BufWriter<Box<dyn Write>>,
    sequence: u64,
    instruction_epoch: u64,
    mode: InstructionTraceMode,
    enabled: bool,
    deferred_error: Option<InstructionTraceError>,
    normalized_slots: Vec<NormalizedTraceSlot>,
    suppressed_instructions: Vec<SuppressedInstruction>,
    pending_writes: Vec<PendingTraceWrites>,
    pending_diffs: Vec<PendingInstructionDiff>,
    pending_stack_updates: Vec<PendingStackTopUpdate>,
    callback_barriers: Vec<TraceCallbackBarrier>,
    projected_stack_tops: Vec<ProjectedStackTop>,
    open_upvalues: Vec<TraceOpenUpvalue>,
    pending_lifecycle_events: Vec<PendingLifecycleEvent>,
    ready_lifecycle_events: Vec<PendingLifecycleEvent>,
}

impl fmt::Debug for InstructionTrace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstructionTrace")
            .field("sequence", &self.sequence)
            .field("instruction_epoch", &self.instruction_epoch)
            .field("mode", &self.mode)
            .field("enabled", &self.enabled)
            .field("deferred_error", &self.deferred_error)
            .field("normalized_slots", &self.normalized_slots.len())
            .field(
                "suppressed_instructions",
                &self.suppressed_instructions.len(),
            )
            .field("pending_writes", &self.pending_writes.len())
            .field("pending_diffs", &self.pending_diffs.len())
            .field("pending_stack_updates", &self.pending_stack_updates.len())
            .field("callback_barriers", &self.callback_barriers.len())
            .field("projected_stack_tops", &self.projected_stack_tops.len())
            .field("open_upvalues", &self.open_upvalues.len())
            .field(
                "pending_lifecycle_events",
                &self.pending_lifecycle_events.len(),
            )
            .field("ready_lifecycle_events", &self.ready_lifecycle_events.len())
            .finish_non_exhaustive()
    }
}

impl InstructionTrace {
    pub(crate) fn new(output: Box<dyn Write>, mode: InstructionTraceMode) -> Self {
        Self {
            output: BufWriter::new(output),
            sequence: 0,
            instruction_epoch: 0,
            mode,
            enabled: true,
            deferred_error: None,
            normalized_slots: Vec::new(),
            suppressed_instructions: Vec::new(),
            pending_writes: Vec::new(),
            pending_diffs: Vec::new(),
            pending_stack_updates: Vec::new(),
            callback_barriers: Vec::new(),
            projected_stack_tops: Vec::new(),
            open_upvalues: Vec::new(),
            pending_lifecycle_events: Vec::new(),
            ready_lifecycle_events: Vec::new(),
        }
    }

    fn write_event(&mut self, line: &str) {
        if !self.enabled || self.deferred_error.is_some() {
            return;
        }
        if let Err(error) = self
            .output
            .write_all(line.as_bytes())
            .and_then(|()| self.output.write_all(b"\n"))
        {
            self.deferred_error = Some(InstructionTraceError::from_io(error));
            self.enabled = false;
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
    }

    pub(crate) fn finish(&mut self) -> Result<InstructionTraceSummary, InstructionTraceError> {
        self.flush_all_lifecycle_events();
        self.enabled = false;
        if self.deferred_error.is_none()
            && let Err(error) = self.output.flush()
        {
            self.deferred_error = Some(InstructionTraceError::from_io(error));
        }
        if let Some(error) = self.deferred_error.clone() {
            return Err(error);
        }
        Ok(InstructionTraceSummary {
            event_count: self.sequence,
        })
    }

    fn record_slot(
        &mut self,
        state: StateHandle,
        slot: usize,
        physical: &Value,
        rendered: TraceRenderedValue,
    ) {
        self.normalized_slots
            .retain(|entry| entry.state != state || entry.slot != slot);
        if rendered.matches(physical) {
            return;
        }
        self.normalized_slots.push(NormalizedTraceSlot {
            state,
            slot,
            physical: TraceValueIdentity::of(physical),
            rendered,
            created_at_epoch: self.instruction_epoch,
        });
    }

    fn rendered_value(&mut self, state: StateHandle, slot: usize, value: &Value) -> Option<Value> {
        let position = self
            .normalized_slots
            .iter()
            .position(|entry| entry.state == state && entry.slot == slot)?;
        if self.normalized_slots[position].physical == TraceValueIdentity::of(value) {
            return Some(self.normalized_slots[position].rendered.value());
        }
        self.normalized_slots.swap_remove(position);
        None
    }

    fn suppress_instruction(&mut self, state: StateHandle, frame: usize, proto: usize, pc: usize) {
        if !self.suppressed_instructions.iter().any(|entry| {
            entry.state == state && entry.frame == frame && entry.proto == proto && entry.pc == pc
        }) {
            self.suppressed_instructions.push(SuppressedInstruction {
                state,
                frame,
                proto,
                pc,
            });
        }
    }

    fn take_instruction_suppression(
        &mut self,
        state: StateHandle,
        frame: usize,
        proto: usize,
        pc: usize,
    ) -> bool {
        let position = self.suppressed_instructions.iter().position(|entry| {
            entry.state == state && entry.frame == frame && entry.proto == proto && entry.pc == pc
        });
        let Some(position) = position else {
            return false;
        };
        self.suppressed_instructions.swap_remove(position);
        true
    }

    fn callback_blocks_epoch(&self, state: StateHandle, instruction_epoch: u64) -> bool {
        self.callback_barriers
            .iter()
            .any(|barrier| barrier.state == state && barrier.instruction_epoch == instruction_epoch)
    }

    fn native_frame_blocks(&self, state: &LuaState, owner_frame: usize) -> bool {
        state.current_ci > owner_frame
            && state
                .call_stack
                .get(owner_frame.saturating_add(1))
                .is_some_and(|frame| frame.proto.is_none())
    }

    fn apply_pending_writes(&mut self, state_identity: StateHandle, state: &LuaState) {
        let mut index = 0;
        while index < self.pending_writes.len() {
            let pending = &self.pending_writes[index];
            if pending.state != state_identity
                || self.callback_blocks_epoch(pending.state, pending.instruction_epoch)
                || self.native_frame_blocks(state, pending.frame)
            {
                index += 1;
                continue;
            }
            let pending = self.pending_writes.swap_remove(index);
            self.normalized_slots.retain(|entry| {
                entry.state != state_identity
                    || entry.created_at_epoch > pending.instruction_epoch
                    || !pending.slots.contains(&entry.slot)
            });
        }
    }

    fn schedule_writes(
        &mut self,
        state: StateHandle,
        instruction_epoch: u64,
        frame: usize,
        slots: Vec<usize>,
    ) {
        if !slots.is_empty() {
            self.pending_writes.push(PendingTraceWrites {
                state,
                instruction_epoch,
                frame,
                slots,
            });
        }
    }

    fn invalidate_written_slots(
        &mut self,
        state: StateHandle,
        instruction_epoch: u64,
        slots: &[usize],
    ) {
        self.normalized_slots.retain(|entry| {
            entry.state != state
                || entry.created_at_epoch > instruction_epoch
                || !slots.contains(&entry.slot)
        });
    }

    fn projected_stack_top(
        &mut self,
        state_identity: StateHandle,
        state: &LuaState,
        max_stack: usize,
    ) -> i32 {
        let frame = state.current_ci;
        if let Some(entry) = self
            .projected_stack_tops
            .iter()
            .find(|entry| entry.state == state_identity && entry.frame == frame)
        {
            return entry.top;
        }
        let top = i32::try_from(max_stack).unwrap_or(i32::MAX);
        self.projected_stack_tops.push(ProjectedStackTop {
            state: state_identity,
            frame,
            top,
        });
        top
    }

    fn apply_stack_top_transition(
        &mut self,
        state_identity: StateHandle,
        state: &LuaState,
        pending: &PendingStackTopUpdate,
    ) -> i32 {
        let current_frame = state.current_ci;
        let current_base = state.current_call_info().base;
        let raw_top = i64::try_from(state.top).unwrap_or(i64::MAX)
            - i64::try_from(current_base).unwrap_or(i64::MAX);
        let raw_top = raw_top.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let frame_capacity = state.current_call_info().top.saturating_sub(current_base);
        let frame_capacity = i32::try_from(frame_capacity).unwrap_or(i32::MAX);

        if opcode::get_opcode(pending.instruction) == opcode::OpCode::RETURN
            && let Some(owner) = pending.callback_owner
        {
            if current_frame <= owner.frame {
                self.projected_stack_tops
                    .retain(|entry| entry.state != state_identity || entry.frame <= owner.frame);
                if let Some(entry) = self
                    .projected_stack_tops
                    .iter_mut()
                    .find(|entry| entry.state == state_identity && entry.frame == owner.frame)
                {
                    entry.top = owner.top;
                } else {
                    self.projected_stack_tops.push(ProjectedStackTop {
                        state: state_identity,
                        frame: owner.frame,
                        top: owner.top,
                    });
                }
            }
            return return_result_count(pending.instruction, pending.stack_top_before)
                .saturating_sub(1);
        }

        let next_top = if current_frame > pending.frame {
            frame_capacity
        } else if current_frame < pending.frame {
            self.projected_stack_tops
                .retain(|entry| entry.state != state_identity || entry.frame <= current_frame);
            raw_top
        } else {
            use opcode::OpCode;
            match opcode::get_opcode(pending.instruction) {
                OpCode::CALL if opcode::get_arg_c(pending.instruction) != 0 => {
                    i32::try_from(pending.max_stack).unwrap_or(i32::MAX)
                }
                OpCode::CALL => raw_top,
                OpCode::VARARG if opcode::get_arg_b(pending.instruction) == 0 => raw_top,
                OpCode::TFORLOOP => i32::try_from(pending.max_stack).unwrap_or(i32::MAX),
                OpCode::SETLIST if opcode::get_arg_b(pending.instruction) == 0 => {
                    i32::try_from(pending.max_stack).unwrap_or(i32::MAX)
                }
                OpCode::TAILCALL => frame_capacity,
                OpCode::RETURN => {
                    let top_before = self
                        .projected_stack_tops
                        .iter()
                        .find(|entry| entry.state == state_identity && entry.frame == current_frame)
                        .map(|entry| entry.top)
                        .unwrap_or(raw_top);
                    return_result_count(pending.instruction, top_before).saturating_sub(1)
                }
                _ => self
                    .projected_stack_tops
                    .iter()
                    .find(|entry| entry.state == state_identity && entry.frame == current_frame)
                    .map(|entry| entry.top)
                    .unwrap_or_else(|| i32::try_from(pending.max_stack).unwrap_or(i32::MAX)),
            }
        };

        if let Some(entry) = self
            .projected_stack_tops
            .iter_mut()
            .find(|entry| entry.state == state_identity && entry.frame == current_frame)
        {
            entry.top = next_top;
        } else {
            self.projected_stack_tops.push(ProjectedStackTop {
                state: state_identity,
                frame: current_frame,
                top: next_top,
            });
        }
        next_top
    }

    fn write_lifecycle_event(&mut self, event: &PendingLifecycleEvent) {
        let mut output = String::from("{\"seq\":");
        output.push_str(&self.sequence.to_string());
        output.push_str(",\"kind\":");
        push_json_string(&mut output, event.kind);
        output.push_str(",\"funcName\":");
        push_json_string(&mut output, &event.function_name);
        output.push_str(",\"source\":");
        push_json_string(&mut output, &event.source);
        output.push_str(",\"line\":");
        output.push_str(&event.line.to_string());
        output.push_str(",\"callDepth\":");
        output.push_str(&event.call_depth.to_string());
        output.push_str(",\"slot\":");
        output.push_str(&event.slot.to_string());
        output.push_str(",\"name\":");
        push_json_string(&mut output, &event.name);
        output.push('}');
        self.write_event(&output);
    }

    fn flush_lifecycle_events(&mut self, state: StateHandle) {
        let mut index = 0;
        while index < self.pending_lifecycle_events.len() {
            if self.pending_lifecycle_events[index].state != state {
                index += 1;
                continue;
            }
            let event = self.pending_lifecycle_events.remove(index);
            self.write_lifecycle_event(&event);
        }
    }

    fn flush_all_lifecycle_events(&mut self) {
        for event in std::mem::take(&mut self.pending_lifecycle_events) {
            self.write_lifecycle_event(&event);
        }
        self.flush_ready_lifecycle_events();
    }

    fn flush_ready_lifecycle_events(&mut self) {
        for event in std::mem::take(&mut self.ready_lifecycle_events) {
            self.write_lifecycle_event(&event);
        }
    }

    fn retire_state(&mut self, state: StateHandle) {
        self.flush_lifecycle_events(state);
        self.normalized_slots.retain(|entry| entry.state != state);
        self.suppressed_instructions
            .retain(|entry| entry.state != state);
        self.pending_writes.retain(|entry| entry.state != state);
        self.pending_diffs.retain(|entry| entry.state != state);
        self.pending_stack_updates
            .retain(|entry| entry.state != state);
        self.callback_barriers.retain(|entry| entry.state != state);
        self.projected_stack_tops
            .retain(|entry| entry.state != state);
        self.open_upvalues.retain(|entry| entry.state != state);
        self.pending_lifecycle_events
            .retain(|entry| entry.state != state);
    }
}

fn trace_state_identity(state: &LuaState) -> Option<StateHandle> {
    state.state_handle()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NormalizedTraceSlot {
    state: StateHandle,
    slot: usize,
    physical: TraceValueIdentity,
    rendered: TraceRenderedValue,
    created_at_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SuppressedInstruction {
    state: StateHandle,
    frame: usize,
    proto: usize,
    pc: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingTraceWrites {
    state: StateHandle,
    instruction_epoch: u64,
    frame: usize,
    slots: Vec<usize>,
}

#[derive(Debug)]
struct PendingInstructionDiff {
    state: StateHandle,
    instruction_epoch: u64,
    base: usize,
    frame: usize,
    max_stack: usize,
    function_name: String,
    source: String,
    line: i32,
    call_depth: i32,
    wanted_results: i32,
    pc: usize,
    instruction: Instruction,
    stack_top_before: i32,
    callback_owner: Option<CallbackStackOwner>,
    local_names: Vec<Option<String>>,
    before: Vec<Value>,
    written_slots: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct PendingStackTopUpdate {
    state: StateHandle,
    instruction_epoch: u64,
    frame: usize,
    max_stack: usize,
    instruction: Instruction,
    stack_top_before: i32,
    callback_owner: Option<CallbackStackOwner>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CallbackStackOwner {
    frame: usize,
    top: i32,
}

/// One Runtime-owned Lua callback that temporarily executes inside an opcode.
/// The epoch identifies the exact pending instruction whose post-state is not
/// observable until its continuation publishes the callback result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TraceCallbackBarrier {
    state: StateHandle,
    instruction_epoch: u64,
    owner: CallbackStackOwner,
}

#[derive(Clone, Copy, Debug)]
struct ProjectedStackTop {
    state: StateHandle,
    frame: usize,
    top: i32,
}

#[derive(Clone, Debug)]
struct TraceOpenUpvalue {
    state: StateHandle,
    absolute_slot: usize,
    slot: i32,
    name: String,
    function_name: String,
    source: String,
    line: i32,
    call_depth: i32,
}

#[derive(Clone, Debug)]
struct PendingLifecycleEvent {
    state: StateHandle,
    kind: &'static str,
    function_name: String,
    source: String,
    line: i32,
    call_depth: i32,
    slot: i32,
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceRenderedValue {
    Nil,
    Number(u64),
}

impl TraceRenderedValue {
    fn matches(self, value: &Value) -> bool {
        match (self, value) {
            (Self::Nil, Value::Nil) => true,
            (Self::Number(expected), Value::Number(actual)) => expected == actual.to_bits(),
            _ => false,
        }
    }

    fn value(self) -> Value {
        match self {
            Self::Nil => Value::Nil,
            Self::Number(bits) => Value::Number(f64::from_bits(bits)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceValueIdentity {
    Nil,
    Boolean(bool),
    LightUserdata(usize),
    Number(u64),
    String(usize),
    Table(usize),
    Function(usize),
    Userdata(usize),
    Thread(usize),
}

impl TraceValueIdentity {
    fn of(value: &Value) -> Self {
        match value {
            Value::Nil => Self::Nil,
            Value::Boolean(value) => Self::Boolean(*value),
            Value::LightUserdata(value) => Self::LightUserdata(value.as_ptr() as usize),
            Value::Number(value) => Self::Number(value.to_bits()),
            Value::String(value) => Self::String(value.as_ptr() as usize),
            Value::Table(value) => Self::Table(value.as_ptr() as usize),
            Value::Function(value) => Self::Function(value.as_ptr() as usize),
            Value::Userdata(value) => Self::Userdata(value.as_ptr() as usize),
            Value::Thread(value) => Self::Thread(value.as_ptr() as usize),
        }
    }
}

/// Project one fixed-C++ logical cleanup range without mutating the VM stack.
pub(crate) fn record_trace_nil_range(state: &LuaState, start: usize, end: usize) {
    let Some(trace) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let clear_end = end.min(state.stack.size());
    let mut trace = trace.borrow_mut();
    for slot in start.min(clear_end)..clear_end {
        let value = state.stack.at(slot).cloned().unwrap_or(Value::Nil);
        trace.record_slot(state_identity, slot, &value, TraceRenderedValue::Nil);
    }
}

/// Drop unfinished Diff instructions owned by frames above an unwind boundary.
/// The native caller's pending CALL remains below that boundary and completes
/// only after its native barrier has been removed.
pub(crate) fn discard_instruction_diffs_above_frame(state: &LuaState, target_ci: usize) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    trace
        .pending_diffs
        .retain(|pending| pending.state != state_identity || pending.frame <= target_ci);
    trace
        .pending_stack_updates
        .retain(|pending| pending.state != state_identity || pending.frame <= target_ci);
}

/// Drop every unfinished Diff instruction for a terminal state error.
pub(crate) fn discard_all_instruction_diffs(state: &LuaState) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    trace
        .pending_diffs
        .retain(|pending| pending.state != state_identity);
    trace
        .pending_stack_updates
        .retain(|pending| pending.state != state_identity);
    trace
        .callback_barriers
        .retain(|barrier| barrier.state != state_identity);
}

/// Hold the currently executing opcode while a Runtime-owned Lua callback
/// runs. The returned token is tied to the instruction epoch, not merely to a
/// frame depth, so callback frames cannot accidentally complete their owner's
/// Diff or consume its Snapshot stack projection.
pub(crate) fn begin_trace_callback_barrier(state: &LuaState) -> Option<TraceCallbackBarrier> {
    begin_trace_callback_barrier_for_frame(state, state.current_ci)
}

pub(crate) fn begin_trace_callback_barrier_for_frame(
    state: &LuaState,
    owner_frame: usize,
) -> Option<TraceCallbackBarrier> {
    let shared = state.instruction_trace.as_ref().cloned()?;
    let state_identity = trace_state_identity(state)?;
    let mut trace = shared.borrow_mut();
    if !trace.enabled {
        return None;
    }
    let instruction_epoch = trace
        .pending_diffs
        .iter()
        .rev()
        .find(|pending| pending.state == state_identity && pending.frame == owner_frame)
        .map(|pending| pending.instruction_epoch)
        .or_else(|| {
            trace
                .pending_stack_updates
                .iter()
                .rev()
                .find(|pending| pending.state == state_identity && pending.frame == owner_frame)
                .map(|pending| pending.instruction_epoch)
        })?;
    let barrier = TraceCallbackBarrier {
        state: state_identity,
        instruction_epoch,
        owner: CallbackStackOwner {
            frame: owner_frame,
            top: trace
                .projected_stack_tops
                .iter()
                .find(|entry| entry.state == state_identity && entry.frame == owner_frame)
                .map(|entry| entry.top)
                .unwrap_or_else(|| {
                    state
                        .call_stack
                        .get(owner_frame)
                        .map(|frame| {
                            i32::try_from(frame.top.saturating_sub(frame.base)).unwrap_or(i32::MAX)
                        })
                        .unwrap_or(0)
                }),
        },
    };
    trace.callback_barriers.push(barrier);
    Some(barrier)
}

/// Release exactly one callback-owned opcode barrier. Callers invoke this only
/// after publishing the callback result (or after abandoning the continuation
/// on error), so the next ordinary trace boundary may observe the owner opcode.
pub(crate) fn end_trace_callback_barrier(state: &LuaState, barrier: Option<TraceCallbackBarrier>) {
    let Some(barrier) = barrier else {
        return;
    };
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if let Some(position) = trace
        .callback_barriers
        .iter()
        .rposition(|entry| *entry == barrier)
    {
        trace.callback_barriers.remove(position);
    }
}

/// Project the fixed C++ numeric-FOR terminating probe without changing the
/// Rust VM's established post-loop register semantics.
pub(crate) fn record_trace_number_override(state: &LuaState, slot: usize, rendered: f64) {
    let Some(trace) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let physical = state.stack.at(slot).cloned().unwrap_or(Value::Nil);
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    trace.borrow_mut().record_slot(
        state_identity,
        slot,
        &physical,
        TraceRenderedValue::Number(rendered.to_bits()),
    );
}

/// The fixed C++ TEST/TESTSET handlers consume their paired JMP internally.
pub(crate) fn record_fused_jump_trace_suppression(
    state: &LuaState,
    proto: GcRef<Proto>,
    pc: usize,
) {
    let Some(trace) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    trace.borrow_mut().suppress_instruction(
        state_identity,
        state.current_ci,
        proto.as_ptr() as usize,
        pc,
    );
}

/// Record the first time a stack slot becomes an open Upvalue. The event is
/// deferred until the CLOSURE instruction's Snapshot/Diff evidence is complete.
pub(crate) fn record_trace_upvalue_open(
    state: &LuaState,
    gc: &GarbageCollector,
    owner_proto: GcRef<Proto>,
    pc: usize,
    absolute_slot: usize,
    name: Option<GcRef<GcString>>,
) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(location) = proto_trace_location(gc, owner_proto, pc) else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let relative_slot = absolute_slot.saturating_sub(state.current_call_info().base);
    let slot = i32::try_from(relative_slot).unwrap_or(i32::MAX);
    let name = name
        .and_then(|name| copy_trace_string(gc, name))
        .unwrap_or_else(|| "?".to_string());
    let mut trace = shared.borrow_mut();
    if !trace.enabled
        || trace
            .open_upvalues
            .iter()
            .any(|entry| entry.state == state_identity && entry.absolute_slot == absolute_slot)
    {
        return;
    }
    trace.open_upvalues.push(TraceOpenUpvalue {
        state: state_identity,
        absolute_slot,
        slot,
        name: name.clone(),
        function_name: location.function_name.clone(),
        source: location.source.clone(),
        line: location.line,
        call_depth: state.nccalls,
    });
    trace.pending_lifecycle_events.push(PendingLifecycleEvent {
        state: state_identity,
        kind: "upvalue-open",
        function_name: location.function_name,
        source: location.source,
        line: location.line,
        call_depth: state.nccalls,
        slot,
        name,
    });
}

/// Record every tracked Upvalue closed at or above `level`. The actual VM
/// close has already succeeded when this helper is called.
pub(crate) fn record_trace_upvalue_closes(state: &LuaState, gc: &GarbageCollector, level: usize) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(proto_ref) = state.current_call_info().proto else {
        return;
    };
    let pc = state.current_call_info().savedpc.unwrap_or(0);
    let Some(location) = proto_trace_location(gc, proto_ref, pc) else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if !trace.enabled {
        return;
    }

    let mut closed = Vec::new();
    let mut index = 0;
    while index < trace.open_upvalues.len() {
        let entry = &trace.open_upvalues[index];
        if entry.state == state_identity && entry.absolute_slot >= level {
            closed.push(trace.open_upvalues.remove(index));
        } else {
            index += 1;
        }
    }
    closed.sort_by_key(|entry| std::cmp::Reverse(entry.absolute_slot));
    for entry in closed {
        trace.pending_lifecycle_events.push(PendingLifecycleEvent {
            state: state_identity,
            kind: "upvalue-close",
            function_name: location.function_name.clone(),
            source: location.source.clone(),
            line: location.line,
            call_depth: state.nccalls,
            slot: entry.slot,
            name: entry.name,
        });
    }
}

/// Record one collector-validated Upvalue close while its owning state is
/// being retired. The opening instruction metadata is retained because no
/// Lua instruction is active at a collector shutdown boundary.
pub(crate) fn record_trace_shutdown_upvalue_close(state: &LuaState, absolute_slot: usize) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    let Some(position) = trace
        .open_upvalues
        .iter()
        .position(|entry| entry.state == state_identity && entry.absolute_slot == absolute_slot)
    else {
        return;
    };
    let entry = trace.open_upvalues.remove(position);
    trace.ready_lifecycle_events.push(PendingLifecycleEvent {
        state: state_identity,
        kind: "upvalue-close",
        function_name: entry.function_name,
        source: entry.source,
        line: entry.line,
        call_depth: entry.call_depth,
        slot: entry.slot,
        name: entry.name,
    });
}

/// Purge all per-state trace bookkeeping before an arena generation advances.
pub(crate) fn retire_trace_state(trace: &SharedInstructionTrace, state: StateHandle) {
    trace.borrow_mut().retire_state(state);
}

#[derive(Debug)]
struct InstructionMetadata {
    source: Option<GcRef<GcString>>,
    line_defined: i32,
    line: i32,
    max_stack: usize,
    local_names: Vec<Option<GcRef<GcString>>>,
}

#[derive(Debug)]
struct TraceLocation {
    function_name: String,
    source: String,
    line: i32,
}

fn proto_trace_location(
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
) -> Option<TraceLocation> {
    let (source_ref, line_defined, line) = gc
        .with_ref(proto_ref, |proto| {
            (proto.source(), proto.line_defined(), proto.line(pc))
        })
        .ok()?;
    let source = source_ref
        .and_then(|source| copy_trace_string(gc, source))
        .unwrap_or_else(|| "?".to_string());
    let function_name = if line_defined > 0 {
        format!("{source}:{line_defined}")
    } else {
        source.clone()
    };
    Some(TraceLocation {
        function_name,
        source,
        line,
    })
}

fn trace_function_name(gc: &GarbageCollector, value: &Value) -> String {
    let Value::Function(function_ref) = value else {
        return "?".to_string();
    };
    let Ok((is_c, proto)) = gc.with_ref(*function_ref, |function: &Function| {
        (function.is_c_function(), function.proto())
    }) else {
        return "?".to_string();
    };
    if is_c {
        return "C function".to_string();
    }
    proto
        .and_then(|proto| proto_trace_location(gc, proto, 0))
        .map(|location| location.function_name)
        .unwrap_or_else(|| "?".to_string())
}

fn emit_control_trace(
    state: &LuaState,
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
    kind: &str,
    function_name: String,
    call_depth: i32,
) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(location) = proto_trace_location(gc, proto_ref, pc) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if !trace.enabled {
        return;
    }
    trace.flush_ready_lifecycle_events();
    let mut event = String::from("{\"seq\":");
    event.push_str(&trace.sequence.to_string());
    event.push_str(",\"kind\":");
    push_json_string(&mut event, kind);
    event.push_str(",\"funcName\":");
    push_json_string(&mut event, &function_name);
    event.push_str(",\"source\":");
    push_json_string(&mut event, &location.source);
    event.push_str(",\"line\":");
    event.push_str(&location.line.to_string());
    event.push_str(",\"callDepth\":");
    event.push_str(&call_depth.to_string());
    event.push('}');
    trace.write_event(&event);
}

/// Emit the pinned C++ CALL-opcode lifecycle event before call preparation.
pub(crate) fn emit_call_trace(
    state: &LuaState,
    gc: &GarbageCollector,
    caller_proto: GcRef<Proto>,
    pc: usize,
    callee: &Value,
) {
    emit_control_trace(
        state,
        gc,
        caller_proto,
        pc,
        "call",
        trace_function_name(gc, callee),
        state.nccalls.saturating_add(1),
    );
}

/// Emit the pinned C++ RETURN-opcode lifecycle event before frame teardown.
pub(crate) fn emit_return_trace(
    state: &LuaState,
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
) {
    let function_name = proto_trace_location(gc, proto_ref, pc)
        .map(|location| location.function_name)
        .unwrap_or_else(|| "?".to_string());
    emit_control_trace(
        state,
        gc,
        proto_ref,
        pc,
        "return",
        function_name,
        state.nccalls,
    );
}

/// Emit a coroutine resume event immediately before the target Lua frame
/// continues at `pc`.
pub(crate) fn emit_coroutine_resume_trace(
    state: &LuaState,
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
) {
    let function_name = proto_trace_location(gc, proto_ref, pc)
        .map(|location| location.function_name)
        .unwrap_or_else(|| "?".to_string());
    emit_control_trace(
        state,
        gc,
        proto_ref,
        pc,
        "resume",
        function_name,
        state.nccalls,
    );
}

/// Emit a coroutine yield event after the yielding CALL instruction has
/// completed and before control returns to the resumer.
pub(crate) fn emit_coroutine_yield_trace(
    state: &LuaState,
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
) {
    let function_name = proto_trace_location(gc, proto_ref, pc)
        .map(|location| location.function_name)
        .unwrap_or_else(|| "?".to_string());
    emit_control_trace(
        state,
        gc,
        proto_ref,
        pc,
        "yield",
        function_name,
        state.nccalls,
    );
}

/// Record an unhandled VM error at the top-level Runtime execution boundary.
pub(crate) fn emit_runtime_error_trace(state: &LuaState, message: &str) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if !trace.enabled {
        return;
    }
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    trace.flush_lifecycle_events(state_identity);
    trace.flush_ready_lifecycle_events();
    let mut event = String::from("{\"seq\":");
    event.push_str(&trace.sequence.to_string());
    event.push_str(",\"kind\":\"error\",\"funcName\":\"?\",\"message\":");
    push_json_string(&mut event, message);
    event.push_str(",\"errorValue\":");
    push_json_string(&mut event, message);
    event.push_str(",\"errorCategory\":\"runtime\",\"source\":\"?\",\"line\":0,\"callDepth\":0}");
    trace.write_event(&event);
}

/// Emit or begin the configured register evidence for one real VM instruction.
pub(crate) fn emit_instruction_snapshot(
    state: &LuaState,
    gc: &GarbageCollector,
    proto_ref: GcRef<Proto>,
    pc: usize,
    instruction: Instruction,
) {
    let Some(trace) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    flush_instruction_diff(state, gc);
    apply_snapshot_stack_top_update(state);
    if !trace.borrow().enabled {
        return;
    }

    let Ok(metadata) = gc.with_ref(proto_ref, |proto| {
        let local_names = (0..proto.max_stack_size() as usize)
            .map(|slot| {
                proto
                    .local_var_info(slot as i32 + 1, pc as i32)
                    .and_then(|local| local.varname)
            })
            .collect();
        InstructionMetadata {
            source: proto.source(),
            line_defined: proto.line_defined(),
            line: proto.line(pc),
            max_stack: proto.max_stack_size() as usize,
            local_names,
        }
    }) else {
        return;
    };

    let source = metadata
        .source
        .and_then(|source| copy_trace_string(gc, source))
        .unwrap_or_else(|| "?".to_string());
    let function_name = if metadata.line_defined > 0 {
        format!("{}:{}", source, metadata.line_defined)
    } else {
        source.clone()
    };
    let base = state.current_call_info().base;
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = trace.borrow_mut();
    if trace.mode == InstructionTraceMode::Snapshot {
        trace.apply_pending_writes(state_identity, state);
    }
    if trace.take_instruction_suppression(
        state_identity,
        state.current_ci,
        proto_ref.as_ptr() as usize,
        pc,
    ) {
        return;
    }

    let local_names = metadata
        .local_names
        .into_iter()
        .map(|name| name.and_then(|name| copy_trace_string(gc, name)))
        .collect::<Vec<_>>();
    let mut before = Vec::with_capacity(metadata.max_stack);
    for slot in 0..metadata.max_stack {
        let absolute_slot = base.saturating_add(slot);
        let physical = state.stack.at(absolute_slot).cloned().unwrap_or(Value::Nil);
        let value = trace
            .rendered_value(state_identity, absolute_slot, &physical)
            .unwrap_or(physical);
        before.push(value);
    }

    let instruction_epoch = trace.instruction_epoch;
    let written_slots = instruction_written_slots(instruction, base, metadata.max_stack);
    trace.flush_ready_lifecycle_events();
    let stack_top = trace.projected_stack_top(state_identity, state, metadata.max_stack);
    let callback_owner = trace
        .callback_barriers
        .iter()
        .rev()
        .find(|barrier| barrier.state == state_identity)
        .map(|barrier| barrier.owner);
    match trace.mode {
        InstructionTraceMode::Snapshot => {
            let mut event = instruction_event_prefix(
                trace.sequence,
                &function_name,
                pc,
                instruction,
                metadata.line,
                &source,
                state.nccalls,
                stack_top,
            );
            event.push_str(",\"registers\":");
            push_register_snapshot(&mut event, gc, &local_names, &before);
            event.push('}');
            trace.schedule_writes(
                state_identity,
                instruction_epoch,
                state.current_ci,
                written_slots,
            );
            trace.pending_stack_updates.push(PendingStackTopUpdate {
                state: state_identity,
                instruction_epoch,
                frame: state.current_ci,
                max_stack: metadata.max_stack,
                instruction,
                stack_top_before: stack_top,
                callback_owner,
            });
            trace.write_event(&event);
        }
        InstructionTraceMode::Diff => {
            trace.pending_diffs.push(PendingInstructionDiff {
                state: state_identity,
                instruction_epoch,
                base,
                frame: state.current_ci,
                max_stack: metadata.max_stack,
                function_name,
                source,
                line: metadata.line,
                call_depth: state.nccalls,
                wanted_results: state.current_call_info().nresults,
                pc,
                instruction,
                stack_top_before: stack_top,
                callback_owner,
                local_names,
                before,
                written_slots,
            });
        }
    }
    trace.instruction_epoch = trace.instruction_epoch.saturating_add(1);
}

/// Complete any instruction-diff event whose post-state is now observable.
pub(crate) fn flush_instruction_diff(state: &LuaState, gc: &GarbageCollector) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if !trace.enabled {
        return;
    }

    while trace.mode == InstructionTraceMode::Diff
        && let Some(position) = trace.pending_diffs.iter().position(|pending| {
            if pending.state != state_identity {
                return false;
            }
            if trace.callback_blocks_epoch(pending.state, pending.instruction_epoch) {
                return false;
            }
            let native_barrier = trace.native_frame_blocks(state, pending.frame);
            !native_barrier
        })
    {
        let pending = trace.pending_diffs.remove(position);
        let event_stack_top = trace.apply_stack_top_transition(
            state_identity,
            state,
            &PendingStackTopUpdate {
                state: pending.state,
                instruction_epoch: pending.instruction_epoch,
                frame: pending.frame,
                max_stack: pending.max_stack,
                instruction: pending.instruction,
                stack_top_before: pending.stack_top_before,
                callback_owner: pending.callback_owner,
            },
        );
        trace.invalidate_written_slots(
            state_identity,
            pending.instruction_epoch,
            &pending.written_slots,
        );

        let simulated_after = simulate_cpp_return_after(&pending)
            .or_else(|| simulate_cpp_lua_call_after(state, gc, &pending));

        let mut changes = String::from("[");
        let mut change_count = 0usize;
        for slot in 0..pending.max_stack {
            let absolute_slot = pending.base.saturating_add(slot);
            let physical = state.stack.at(absolute_slot).cloned().unwrap_or(Value::Nil);
            let after = if let Some(value) = simulated_after
                .as_ref()
                .and_then(|registers| registers.get(slot))
            {
                value.clone()
            } else {
                trace
                    .rendered_value(state_identity, absolute_slot, &physical)
                    .unwrap_or(physical)
            };
            let old = pending.before.get(slot).cloned().unwrap_or(Value::Nil);
            if TraceValueIdentity::of(&old) == TraceValueIdentity::of(&after) {
                continue;
            }
            if change_count > 0 {
                changes.push(',');
            }
            change_count += 1;
            changes.push_str("{\"slot\":");
            changes.push_str(&slot.to_string());
            changes.push_str(",\"name\":");
            match pending
                .local_names
                .get(slot)
                .and_then(|name| name.as_deref())
            {
                Some(name) => push_json_string(&mut changes, name),
                None => changes.push_str("null"),
            }
            changes.push_str(",\"old\":");
            push_trace_value(&mut changes, gc, &old);
            changes.push_str(",\"new\":");
            push_trace_value(&mut changes, gc, &after);
            changes.push_str(",\"oldType\":");
            push_json_string(&mut changes, value_type_name(&old));
            changes.push_str(",\"newType\":");
            push_json_string(&mut changes, value_type_name(&after));
            changes.push('}');
        }
        changes.push(']');

        let mut event = instruction_event_prefix(
            trace.sequence,
            &pending.function_name,
            pending.pc,
            pending.instruction,
            pending.line,
            &pending.source,
            pending.call_depth,
            event_stack_top,
        );
        event.push_str(",\"changedRegisters\":");
        event.push_str(&changes);
        event.push('}');
        trace.write_event(&event);
    }
    trace.flush_lifecycle_events(state_identity);
    trace.flush_ready_lifecycle_events();
}

fn return_result_count(instruction: Instruction, stack_top_before: i32) -> i32 {
    let b = opcode::get_arg_b(instruction);
    if b == 0 {
        stack_top_before
            .saturating_sub(opcode::get_arg_a(instruction))
            .max(0)
    } else {
        b.saturating_sub(1)
    }
}

fn simulate_cpp_return_after(pending: &PendingInstructionDiff) -> Option<Vec<Value>> {
    if opcode::get_opcode(pending.instruction) != opcode::OpCode::RETURN {
        return None;
    }
    let actual_results = return_result_count(pending.instruction, pending.stack_top_before);
    let retained_results = if pending.call_depth > 1 && pending.wanted_results >= 0 {
        pending.wanted_results
    } else {
        actual_results
    } as usize;
    let a = opcode::get_arg_a(pending.instruction).max(0) as usize;
    let mut after = vec![Value::Nil; pending.max_stack];
    // Auditing a top-level multi-return against the fixed C++ executable
    // disproved the hypothesis that result zero belongs in this projected
    // register window. Preserve the observed oracle contract: result one is
    // projected into slot zero and result zero remains outside the window.
    for result in 1..retained_results {
        let source = a.saturating_add(result);
        let destination = result - 1;
        if let Some(slot) = after.get_mut(destination) {
            *slot = pending.before.get(source).cloned().unwrap_or(Value::Nil);
        }
    }
    Some(after)
}

fn simulate_cpp_lua_call_after(
    state: &LuaState,
    gc: &GarbageCollector,
    pending: &PendingInstructionDiff,
) -> Option<Vec<Value>> {
    if opcode::get_opcode(pending.instruction) != opcode::OpCode::CALL
        || state.current_ci <= pending.frame
    {
        return None;
    }
    let proto = state.current_call_info().proto?;
    let (num_params, is_vararg, max_stack) = gc
        .with_ref(proto, |proto| {
            (
                proto.num_params() as usize,
                proto.is_vararg(),
                proto.max_stack_size() as usize,
            )
        })
        .ok()?;
    let a = opcode::get_arg_a(pending.instruction).max(0) as usize;
    let actual_args = state.current_call_info().nargs.max(0) as usize;
    let adjusted_args = actual_args.max(num_params);
    let cpp_base = if is_vararg {
        a.saturating_add(1).saturating_add(adjusted_args)
    } else {
        a.saturating_add(1)
    };
    let mut after = pending.before.clone();
    // A debug/metamethod callback can mutate caller locals or open Upvalues
    // while the CALL opcode is suspended. Fixed C++ preserves registers before
    // A and only rewrites the CALL/result window from A onward.
    for slot in 0..a.min(after.len()) {
        let absolute_slot = pending.base.saturating_add(slot);
        let physical = state.stack.at(absolute_slot).cloned().unwrap_or(Value::Nil);
        after[slot] = physical;
    }

    if is_vararg {
        for index in 0..num_params {
            let source = a.saturating_add(1).saturating_add(index);
            let destination = cpp_base.saturating_add(index);
            let value = pending.before.get(source).cloned().unwrap_or(Value::Nil);
            if let Some(slot) = after.get_mut(destination) {
                *slot = value;
            }
            if let Some(slot) = after.get_mut(source) {
                *slot = Value::Nil;
            }
        }
    } else {
        for index in actual_args..num_params {
            if let Some(slot) = after.get_mut(cpp_base.saturating_add(index)) {
                *slot = Value::Nil;
            }
        }
    }

    for slot in cpp_base.saturating_add(num_params)..cpp_base.saturating_add(max_stack) {
        if let Some(value) = after.get_mut(slot) {
            *value = Value::Nil;
        }
    }
    Some(after)
}

fn apply_snapshot_stack_top_update(state: &LuaState) {
    let Some(shared) = state.instruction_trace.as_ref().cloned() else {
        return;
    };
    let Some(state_identity) = trace_state_identity(state) else {
        return;
    };
    let mut trace = shared.borrow_mut();
    if !trace.enabled || trace.mode != InstructionTraceMode::Snapshot {
        return;
    }
    while let Some(position) = trace
        .pending_stack_updates
        .iter()
        .position(|pending| {
            pending.state == state_identity
                && pending.callback_owner.is_some()
                && !trace.callback_blocks_epoch(pending.state, pending.instruction_epoch)
                && !trace.native_frame_blocks(state, pending.frame)
        })
        .or_else(|| {
            trace.pending_stack_updates.iter().position(|pending| {
                pending.state == state_identity
                    && !trace.callback_blocks_epoch(pending.state, pending.instruction_epoch)
                    && !trace.native_frame_blocks(state, pending.frame)
            })
        })
    {
        let pending = trace.pending_stack_updates.remove(position);
        trace.apply_stack_top_transition(state_identity, state, &pending);
    }
}

#[allow(clippy::too_many_arguments)]
fn instruction_event_prefix(
    sequence: u64,
    function_name: &str,
    pc: usize,
    instruction: Instruction,
    line: i32,
    source: &str,
    call_depth: i32,
    stack_top: i32,
) -> String {
    let mut event = String::new();
    event.push_str("{\"seq\":");
    event.push_str(&sequence.to_string());
    event.push_str(",\"kind\":\"instruction\",\"funcName\":");
    push_json_string(&mut event, function_name);
    event.push_str(",\"pc\":");
    event.push_str(&pc.to_string());
    event.push_str(",\"op\":");
    push_json_string(
        &mut event,
        opcode::get_op_name(opcode::get_opcode(instruction)),
    );
    event.push_str(",\"a\":");
    event.push_str(&opcode::get_arg_a(instruction).to_string());
    event.push_str(",\"b\":");
    event.push_str(&opcode::get_arg_b(instruction).to_string());
    event.push_str(",\"c\":");
    event.push_str(&opcode::get_arg_c(instruction).to_string());
    event.push_str(",\"bx\":");
    event.push_str(&opcode::get_arg_bx(instruction).to_string());
    event.push_str(",\"sbx\":");
    event.push_str(&opcode::get_arg_sbx(instruction).to_string());
    event.push_str(",\"line\":");
    event.push_str(&line.to_string());
    event.push_str(",\"source\":");
    push_json_string(&mut event, source);
    event.push_str(",\"callDepth\":");
    event.push_str(&call_depth.to_string());
    event.push_str(",\"stackTop\":");
    event.push_str(&stack_top.to_string());
    event
}

fn push_register_snapshot(
    output: &mut String,
    gc: &GarbageCollector,
    local_names: &[Option<String>],
    registers: &[Value],
) {
    output.push('[');
    for (slot, value) in registers.iter().enumerate() {
        if slot > 0 {
            output.push(',');
        }
        output.push_str("{\"slot\":");
        output.push_str(&slot.to_string());
        output.push_str(",\"name\":");
        match local_names.get(slot).and_then(|name| name.as_deref()) {
            Some(name) => push_json_string(output, name),
            None => output.push_str("null"),
        }
        output.push_str(",\"value\":");
        push_trace_value(output, gc, value);
        output.push_str(",\"type\":");
        push_json_string(output, value_type_name(value));
        output.push('}');
    }
    output.push(']');
}

fn instruction_written_slots(
    instruction: Instruction,
    base: usize,
    max_stack: usize,
) -> Vec<usize> {
    use opcode::OpCode;

    let op = opcode::get_opcode(instruction);
    let a = opcode::get_arg_a(instruction).max(0) as usize;
    let b = opcode::get_arg_b(instruction).max(0) as usize;
    let c = opcode::get_arg_c(instruction).max(0) as usize;
    let range = |start: usize, count: usize| {
        (start..start.saturating_add(count).min(max_stack))
            .map(|slot| base.saturating_add(slot))
            .collect::<Vec<_>>()
    };

    match op {
        OpCode::MOVE
        | OpCode::LOADK
        | OpCode::GETUPVAL
        | OpCode::GETGLOBAL
        | OpCode::GETTABLE
        | OpCode::NEWTABLE
        | OpCode::ADD
        | OpCode::SUB
        | OpCode::MUL
        | OpCode::DIV
        | OpCode::MOD
        | OpCode::POW
        | OpCode::UNM
        | OpCode::NOT
        | OpCode::LEN
        | OpCode::CONCAT
        | OpCode::CLOSURE
        | OpCode::FORPREP
        | OpCode::LOADBOOL => range(a, 1),
        OpCode::LOADNIL => range(a, b.saturating_sub(a).saturating_add(1)),
        OpCode::SELF => range(a, 2),
        OpCode::TESTSET => range(a, 1),
        OpCode::CALL => {
            if c == 0 {
                range(a, max_stack.saturating_sub(a))
            } else {
                range(a, c.saturating_sub(1))
            }
        }
        OpCode::FORLOOP => vec![base.saturating_add(a), base.saturating_add(a + 3)],
        OpCode::TFORLOOP => range(a + 2, c.saturating_add(1)),
        OpCode::VARARG => {
            if b == 0 {
                range(a, max_stack.saturating_sub(a))
            } else {
                range(a, b.saturating_sub(1))
            }
        }
        OpCode::SETGLOBAL
        | OpCode::SETUPVAL
        | OpCode::SETTABLE
        | OpCode::JMP
        | OpCode::EQ
        | OpCode::LT
        | OpCode::LE
        | OpCode::TEST
        | OpCode::TAILCALL
        | OpCode::RETURN
        | OpCode::SETLIST
        | OpCode::CLOSE => Vec::new(),
    }
}

fn copy_trace_string(gc: &GarbageCollector, value: GcRef<GcString>) -> Option<String> {
    gc.with_string_bytes(value, |bytes| String::from_utf8_lossy(bytes).into_owned())
        .ok()
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::LightUserdata(_) => "lightuserdata",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::Userdata(_) => "userdata",
        Value::Thread(_) => "thread",
    }
}

fn push_trace_value(output: &mut String, gc: &GarbageCollector, value: &Value) {
    match value {
        Value::Nil => output.push_str("null"),
        Value::Boolean(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) if value.is_nan() => push_json_string(output, "NaN"),
        Value::Number(value) if value.is_infinite() && value.is_sign_positive() => {
            push_json_string(output, "Infinity");
        }
        Value::Number(value) if value.is_infinite() => push_json_string(output, "-Infinity"),
        Value::Number(value) if value.fract() == 0.0 && value.abs() < 1e15 => {
            output.push_str(&format!("{value:.0}"));
        }
        Value::Number(value) => output.push_str(&format!("{value:.14}")),
        Value::String(value) => {
            let text = copy_trace_string(gc, *value).unwrap_or_default();
            push_json_string(output, &text);
        }
        Value::Table(value) => push_identity(output, "table", value.as_ptr()),
        Value::Function(value) => push_identity(output, "function", value.as_ptr()),
        Value::Userdata(value) => push_identity(output, "userdata", value.as_ptr()),
        Value::Thread(value) => push_identity(output, "thread", value.as_ptr()),
        Value::LightUserdata(value) => push_identity(output, "lightuserdata", value.as_ptr()),
    }
}

fn push_identity<T>(output: &mut String, kind: &str, pointer: *const T) {
    push_json_string(output, &format!("{kind}:{pointer:p}"));
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < '\u{0020}' => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::num::NonZeroU64;

    use lua_core::state_handle::StateHandleIssuer;

    use super::*;

    fn test_state_handle(slot: usize) -> StateHandle {
        StateHandleIssuer::try_new()
            .expect("test runtime identity should be available")
            .issue(
                slot,
                NonZeroU64::new(1).expect("test generation should be non-zero"),
            )
    }

    fn pending_return(call_depth: i32, wanted_results: i32) -> PendingInstructionDiff {
        PendingInstructionDiff {
            state: test_state_handle(0),
            instruction_epoch: 0,
            base: 7,
            frame: usize::from(call_depth > 1),
            max_stack: 6,
            function_name: "return-test".to_string(),
            source: "=return-test".to_string(),
            line: 1,
            call_depth,
            wanted_results,
            pc: 0,
            instruction: opcode::create_abc(opcode::OpCode::RETURN, 2, 4, 0),
            stack_top_before: 5,
            callback_owner: None,
            local_names: vec![None; 6],
            before: vec![
                Value::Number(90.0),
                Value::Number(91.0),
                Value::Number(10.0),
                Value::Number(20.0),
                Value::Number(30.0),
                Value::Number(92.0),
            ],
            written_slots: Vec::new(),
        }
    }

    #[test]
    fn top_level_return_diff_preserves_the_audited_cpp_result_window() {
        let pending = pending_return(1, 3);

        assert_eq!(
            simulate_cpp_return_after(&pending),
            Some(vec![
                Value::Number(20.0),
                Value::Number(30.0),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn nested_return_diff_projects_only_results_inside_the_callee_window() {
        let pending = pending_return(2, 2);

        assert_eq!(
            simulate_cpp_return_after(&pending),
            Some(vec![
                Value::Number(20.0),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn fused_jump_suppression_is_owned_by_the_recording_frame() {
        let state = test_state_handle(4);
        let mut trace = InstructionTrace::new(Box::new(io::sink()), InstructionTraceMode::Diff);

        trace.suppress_instruction(state, 2, 0x1234, 9);

        assert!(!trace.take_instruction_suppression(state, 3, 0x1234, 9));
        assert!(trace.take_instruction_suppression(state, 2, 0x1234, 9));
        assert!(!trace.take_instruction_suppression(state, 2, 0x1234, 9));
    }
}

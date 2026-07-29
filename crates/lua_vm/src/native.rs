//! Runtime-native request protocol.
//!
//! The VM may suspend at a sealed native operation without borrowing another
//! `LuaState`. The current state publishes one owned request, the VM seals the
//! deferred call metadata, and `Runtime` takes the request only after the state
//! turn ends.

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::state_handle::StateHandle;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use crate::state::{CallInfo, LuaState, Stack, ThreadStatus};

/// Identifier for one scoped Runtime-native request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeRequestId(u64);

impl NativeRequestId {
    pub(crate) fn new(value: u64) -> Self {
        debug_assert_ne!(value, 0);
        Self(value)
    }
}

/// Lua-visible result envelope requested by the native closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeEnvelope {
    Resume,
    Wrap,
    ProtectedResume,
    ProtectedWrap,
}

/// VM work that must run after a deferred native call publishes its results.
#[derive(Clone, Debug, Default)]
pub(crate) enum DeferredVmContinuation {
    /// Ordinary CALL has no opcode-local post-processing.
    #[default]
    Call,
    /// TFORLOOP must update its control register or skip the loop body.
    GenericFor { base: usize, register: usize },
}

/// Metadata needed to finish a native call after its target state stops.
#[derive(Clone, Debug)]
pub(crate) struct DeferredNativeCall {
    pub request_id: NativeRequestId,
    pub func_pos: usize,
    pub nargs: usize,
    pub wanted_results: Option<usize>,
    pub saved_ci: usize,
    pub saved_top: usize,
    pub caller_proto: Option<GcRef<Proto>>,
    pub caller_pc: Option<usize>,
    pub continuation: DeferredVmContinuation,
    pub snapshot: StateContinuationSnapshot,
}

/// Execution-owned state restored only for the C++-oracle Normal-ancestor
/// continuation replay. Runtime identity, services, roots, and arena pointers
/// deliberately remain outside this snapshot.
#[derive(Clone, Debug)]
pub(crate) struct StateContinuationSnapshot {
    stack: Stack,
    top: usize,
    call_stack: Vec<CallInfo>,
    current_ci: usize,
    status: ThreadStatus,
    nccalls: i32,
    allow_yield: u16,
    yielded_values: Vec<Value>,
    yield_result_base: Option<usize>,
    yield_wanted_results: Option<usize>,
    last_error: Option<Value>,
}

impl StateContinuationSnapshot {
    pub(crate) fn capture(state: &LuaState) -> Self {
        Self {
            stack: state.stack.clone(),
            top: state.top,
            call_stack: state.call_stack.clone(),
            current_ci: state.current_ci,
            status: state.status,
            nccalls: state.nccalls,
            allow_yield: state.allow_yield,
            yielded_values: state.yielded_values.clone(),
            yield_result_base: state.yield_result_base,
            yield_wanted_results: state.yield_wanted_results,
            last_error: state.last_error.clone(),
        }
    }

    pub(crate) fn restore(&self, state: &mut LuaState) {
        state.stack = self.stack.clone();
        state.top = self.top;
        state.call_stack.clone_from(&self.call_stack);
        state.current_ci = self.current_ci;
        state.status = self.status;
        state.nccalls = self.nccalls;
        state.allow_yield = self.allow_yield;
        state.yielded_values.clone_from(&self.yielded_values);
        state.yield_result_base = self.yield_result_base;
        state.yield_wanted_results = self.yield_wanted_results;
        state.last_error.clone_from(&self.last_error);
    }

    pub(crate) fn top(&self) -> usize {
        self.top
    }

    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        let initialized = self.stack.initialized_values();
        let active_limit = self
            .call_stack
            .iter()
            .take(self.current_ci + 1)
            .map(|call| call.top)
            .max()
            .unwrap_or(self.top)
            .max(self.top)
            .min(initialized.len());
        for value in &initialized[..active_limit] {
            gc.mark_value(value);
        }
        for call in self.call_stack.iter().take(self.current_ci + 1) {
            if let Some(proto) = call.proto {
                gc.mark_registered(proto);
            }
            for value in &call.varargs {
                gc.mark_value(value);
            }
        }
        for value in &self.yielded_values {
            gc.mark_value(value);
        }
        if let Some(error) = &self.last_error {
            gc.mark_value(error);
        }
    }
}

/// Owned transfer published by `coroutine.resume` or a wrap runner.
#[derive(Clone, Debug)]
pub(crate) struct ResumeRequest {
    pub id: NativeRequestId,
    pub thread: GcRef<Thread>,
    pub target: StateHandle,
    pub args: Vec<Value>,
    pub envelope: ResumeEnvelope,
    pub deferred: Option<DeferredNativeCall>,
}

impl ResumeRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&Value::Thread(self.thread));
        for value in &self.args {
            gc.mark_value(value);
        }
        if let Some(deferred) = &self.deferred {
            if let Some(proto) = deferred.caller_proto {
                gc.mark_registered(proto);
            }
            deferred.snapshot.seed_roots(gc);
        }
    }
}

/// Lua-visible result shape for a completed full collection request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FullCollectionResult {
    Collect,
    StepComplete,
}

impl FullCollectionResult {
    pub(crate) fn values(self) -> Vec<Value> {
        match self {
            Self::Collect => vec![Value::Number(0.0)],
            Self::StepComplete => vec![Value::Boolean(true)],
        }
    }
}

/// Owned transfer published by a destructive `collectgarbage` mode.
#[derive(Clone, Debug)]
pub(crate) struct FullCollectionRequest {
    pub id: NativeRequestId,
    pub result: FullCollectionResult,
    pub protected: bool,
    pub deferred: Option<DeferredNativeCall>,
}

impl FullCollectionRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        if let Some(deferred) = &self.deferred {
            if let Some(proto) = deferred.caller_proto {
                gc.mark_registered(proto);
            }
            deferred.snapshot.seed_roots(gc);
        }
    }
}

/// One sealed Runtime-native mailbox payload.
#[derive(Clone, Debug)]
pub(crate) enum RuntimeRequest {
    Resume(ResumeRequest),
    FullCollection(FullCollectionRequest),
}

impl RuntimeRequest {
    pub(crate) fn id(&self) -> NativeRequestId {
        match self {
            Self::Resume(request) => request.id,
            Self::FullCollection(request) => request.id,
        }
    }

    pub(crate) fn deferred(&self) -> Option<&DeferredNativeCall> {
        match self {
            Self::Resume(request) => request.deferred.as_ref(),
            Self::FullCollection(request) => request.deferred.as_ref(),
        }
    }

    pub(crate) fn deferred_mut(&mut self) -> &mut Option<DeferredNativeCall> {
        match self {
            Self::Resume(request) => &mut request.deferred,
            Self::FullCollection(request) => &mut request.deferred,
        }
    }
}

/// Result transferred from a stopped target back to its deferred caller.
#[derive(Clone, Debug)]
pub(crate) enum ResumeResponse {
    Success(Vec<Value>),
    Error(Value),
}

impl ResumeResponse {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        match self {
            Self::Success(values) => {
                for value in values {
                    gc.mark_value(value);
                }
            }
            Self::Error(error) => gc.mark_value(error),
        }
    }
}

/// One VM suspension needed to access an open Upvalue owned by another state.
///
/// Runtime releases the requester turn before resolving `owner`, performs the
/// slot access, and then resumes `requester` at the following opcode.
#[derive(Clone, Debug)]
pub struct UpvalueAccessRequest {
    pub(crate) requester: StateHandle,
    pub(crate) upvalue: GcRef<Upvalue>,
    pub(crate) owner: StateHandle,
    pub(crate) stack_index: usize,
    pub(crate) operation: UpvalueAccessOperation,
}

impl UpvalueAccessRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_registered(self.upvalue);
        if let UpvalueAccessOperation::Write { value } = &self.operation {
            gc.mark_value(value);
        }
    }
}

/// Slot operation carried by an [`UpvalueAccessRequest`].
#[derive(Clone, Debug)]
pub enum UpvalueAccessOperation {
    /// Copy the owner slot into this requester stack destination.
    Read { destination: usize },
    /// Replace the owner slot with an owned value.
    Write { value: Value },
}

/// Failure to publish through the scoped native mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRequestPublishError {
    ScopeUnavailable,
    MailboxOccupied,
    IdExhausted,
}

//! Runtime-native request protocol.
//!
//! The VM may suspend at a sealed native operation without borrowing another
//! `LuaState`. The current state publishes one owned request, the VM seals the
//! deferred call metadata, and `Runtime` takes the request only after the state
//! turn ends.

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::state_handle::StateHandle;
use lua_core::table::Table;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use crate::trace::TraceCallbackBarrier;

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
    /// A Runtime-native TAILCALL must return its completed result window from
    /// the suspended Lua caller instead of executing the opcode again.
    TailCall,
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

    pub(crate) fn allow_yield(&self) -> u16 {
        self.allow_yield
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
    Step { size: i32 },
}

impl FullCollectionResult {
    pub(crate) fn values(self, cycle_finished: bool) -> Vec<Value> {
        match self {
            Self::Collect => vec![Value::Number(0.0)],
            Self::Step { .. } => vec![Value::Boolean(cycle_finished)],
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

/// Owned transfer published by a debug-library open-Upvalue operation.
#[derive(Clone, Debug)]
pub(crate) struct DebugUpvalueRequest {
    pub id: NativeRequestId,
    pub requester: StateHandle,
    pub upvalue: GcRef<Upvalue>,
    pub owner: StateHandle,
    pub stack_index: usize,
    pub name: Value,
    pub operation: DebugUpvalueOperation,
    pub protected: bool,
    pub deferred: Option<DeferredNativeCall>,
}

impl DebugUpvalueRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_registered(self.upvalue);
        gc.mark_value(&self.name);
        if let DebugUpvalueOperation::Write { value } = &self.operation {
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

/// Lua debug-library operation carried by a [`DebugUpvalueRequest`].
#[derive(Clone, Debug)]
pub(crate) enum DebugUpvalueOperation {
    Read,
    Write { value: Value },
}

/// Result policy for one Runtime-owned protected helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectedCallKind {
    PCall,
    XPCall,
}

/// Owned `pcall`/`xpcall` invocation transferred to the Runtime scheduler.
#[derive(Clone, Debug)]
pub(crate) struct ProtectedCallRequest {
    pub id: NativeRequestId,
    pub kind: ProtectedCallKind,
    pub function: Value,
    pub args: Vec<Value>,
    pub handler: Option<Value>,
    pub deferred: Option<DeferredNativeCall>,
}

impl ProtectedCallRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&self.function);
        for value in &self.args {
            gc.mark_value(value);
        }
        if let Some(handler) = &self.handler {
            gc.mark_value(handler);
        }
        if let Some(deferred) = &self.deferred {
            if let Some(proto) = deferred.caller_proto {
                gc.mark_registered(proto);
            }
            deferred.snapshot.seed_roots(gc);
        }
    }
}

/// Result/publication policy for one ordinary stdlib callback helper.
///
/// Each variant owns a distinct Lua-visible contract. This deliberately stays
/// separate from [`ProtectedCallKind`], whose envelope belongs only to
/// `pcall`/`xpcall`.
#[derive(Clone, Debug)]
pub(crate) enum CallbackHelperKind {
    /// `tostring(value)` invoking `value`'s `__tostring` metamethod.
    ToString,
    /// `load(reader, chunk_name)` accumulating bytes over repeated reader calls.
    LoadReader {
        chunk_name: Vec<u8>,
        source: Vec<u8>,
        terminal_error: Option<String>,
    },
    /// `table.sort(table, comparator?)` retaining its transactional working copy
    /// and exact merge cursor between comparator turns.
    TableSort {
        state: Box<TableSortState>,
        terminal_error: Option<String>,
    },
    /// `dofile` executing its compiled chunk and forwarding all results.
    DoFile,
    /// `table.foreach[i]` retaining its exact traversal cursor between calls.
    TableForeach { state: Box<TableForeachState> },
    /// `require` retaining its `package.loaded` publication target and module key.
    PackageRequire {
        loaded: GcRef<Table>,
        module_key: Value,
    },
    /// `module` retaining the ordered option callbacks still to execute.
    PackageModuleOptions {
        state: Box<PackageModuleOptionsState>,
        terminal_error: Option<String>,
    },
    /// `string.gsub` retaining immutable matches and incremental output across
    /// function replacements or table `__index` callbacks.
    Gsub {
        state: Box<GsubState>,
        terminal_error: Option<String>,
    },
    /// One Lua debug-hook event plus the exact VM boundary resumed afterwards.
    DebugHook {
        stop_ci: usize,
        call_pos: usize,
        saved_top: usize,
        trace_barrier: Option<TraceCallbackBarrier>,
        continuation: Box<DebugHookContinuation>,
    },
    /// One opcode-local metamethod call plus its exact VM result publication.
    VmMetamethod {
        stop_ci: usize,
        call_pos: usize,
        saved_top: usize,
        trace_barrier: Option<TraceCallbackBarrier>,
        continuation: VmMetamethodContinuation,
    },
}

/// VM action performed after an opcode metamethod returns its first result.
#[derive(Clone, Debug)]
pub(crate) enum VmMetamethodContinuation {
    /// Store the first callback result and advance beyond the suspended opcode.
    StoreResult {
        proto: GcRef<Proto>,
        pc: usize,
        destination: usize,
    },
    /// Restore SELF's receiver register after callback scratch-stack cleanup,
    /// then store the resolved method in the adjacent destination register.
    StoreSelfResult {
        proto: GcRef<Proto>,
        pc: usize,
        destination: usize,
        self_destination: usize,
        object: Value,
    },
    /// Ignore callback results and advance beyond the suspended opcode.
    CompleteSet { proto: GcRef<Proto>, pc: usize },
    /// Apply a comparison result and optionally skip the following instruction.
    Compare {
        proto: GcRef<Proto>,
        pc: usize,
        expected: bool,
        invert_result: bool,
    },
    /// Store one concat result, then fold the remaining absolute stack slots.
    Concat {
        proto: GcRef<Proto>,
        pc: usize,
        destination: usize,
        next_slot: usize,
        end_slot: usize,
        stack_limit: usize,
    },
}

impl VmMetamethodContinuation {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        let proto = match self {
            Self::StoreResult { proto, .. }
            | Self::StoreSelfResult { proto, .. }
            | Self::CompleteSet { proto, .. }
            | Self::Compare { proto, .. }
            | Self::Concat { proto, .. } => *proto,
        };
        gc.mark_registered(proto);
        if let Self::StoreSelfResult { object, .. } = self {
            gc.mark_value(object);
        }
    }
}

/// Runtime-owned traversal state for `table.foreach` and `table.foreachi`.
#[derive(Clone, Debug)]
pub(crate) struct TableForeachState {
    pub table: GcRef<Table>,
    pub callback: Value,
    pub cursor: TableForeachCursor,
}

impl TableForeachState {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&Value::Table(self.table));
        gc.mark_value(&self.callback);
        if let TableForeachCursor::Pairs { key } = &self.cursor {
            gc.mark_value(key);
        }
    }
}

/// Exact next element selected by one table traversal helper.
#[derive(Clone, Debug)]
pub(crate) enum TableForeachCursor {
    Pairs { key: Value },
    Indices { next: usize, limit: usize },
}

/// Runtime-owned ordered callback state for Lua 5.1 `module` options.
#[derive(Clone, Debug)]
pub(crate) struct PackageModuleOptionsState {
    pub module: GcRef<Table>,
    pub options: Vec<Value>,
    pub next: usize,
}

impl PackageModuleOptionsState {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&Value::Table(self.module));
        for option in &self.options {
            gc.mark_value(option);
        }
    }
}

/// Runtime-owned continuation for callback-capable `string.gsub` replacements.
#[derive(Clone, Debug)]
pub(crate) struct GsubState {
    pub replacement: GsubReplacement,
    pub steps: Vec<GsubStep>,
    pub next: usize,
    pub output: Vec<u8>,
    pub suffix: Vec<u8>,
    pub completed: bool,
}

impl GsubState {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        match &self.replacement {
            GsubReplacement::Function(function) => gc.mark_value(function),
            GsubReplacement::Table { table, index_key } => {
                gc.mark_value(&Value::Table(*table));
                gc.mark_value(index_key);
            }
        }
        for step in &self.steps {
            for argument in &step.arguments {
                gc.mark_value(argument);
            }
            gc.mark_value(&step.table_key);
        }
    }
}

/// Callback-capable replacement source for one `string.gsub` activation.
#[derive(Clone, Debug)]
pub(crate) enum GsubReplacement {
    Function(Value),
    Table {
        table: GcRef<Table>,
        index_key: Value,
    },
}

/// One immutable match selected by the stdlib pattern engine.
#[derive(Clone, Debug)]
pub(crate) struct GsubStep {
    pub prefix: Vec<u8>,
    pub matched: Vec<u8>,
    pub arguments: Vec<Value>,
    pub table_key: Value,
}

/// Native call frame retained while its `call` or `return` hook executes.
#[derive(Clone, Debug)]
pub(crate) struct DebugHookNativeCall {
    pub func_pos: usize,
    pub nargs: usize,
    pub wanted_results: Option<usize>,
    pub saved_ci: usize,
    pub saved_top: usize,
    pub caller_proto: Option<GcRef<Proto>>,
    pub caller_pc: Option<usize>,
    pub continuation: DeferredVmContinuation,
}

/// VM operation paused at a debug-hook event boundary.
#[derive(Clone, Debug)]
pub(crate) enum DebugHookContinuation {
    /// Enter a freshly pushed Lua frame and still run its first instruction hooks.
    LuaCall { proto: GcRef<Proto> },
    /// Resume this exact instruction without dispatching its hooks a second time.
    ResumeAt { proto: GcRef<Proto>, pc: usize },
    /// A count hook completed; line-hook selection for the same instruction is next.
    InstructionAfterCount { proto: GcRef<Proto>, pc: usize },
    /// Finish a Lua RETURN after its return/tail-return hook sequence.
    LuaReturn {
        call_info: CallInfo,
        results: Vec<Value>,
        remaining_tail_returns: i32,
    },
    /// Invoke a native function after its call hook.
    NativeCall { frame: DebugHookNativeCall },
    /// Publish native results after its return hook.
    NativeReturn {
        frame: DebugHookNativeCall,
        result_count: i32,
    },
}

impl DebugHookContinuation {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        match self {
            Self::LuaCall { proto }
            | Self::ResumeAt { proto, .. }
            | Self::InstructionAfterCount { proto, .. } => {
                gc.mark_registered(*proto);
            }
            Self::LuaReturn {
                call_info, results, ..
            } => {
                if let Some(proto) = call_info.proto {
                    gc.mark_registered(proto);
                }
                for value in call_info.varargs.iter().chain(results) {
                    gc.mark_value(value);
                }
            }
            Self::NativeCall { frame } | Self::NativeReturn { frame, .. } => {
                if let Some(proto) = frame.caller_proto {
                    gc.mark_registered(proto);
                }
            }
        }
    }
}

/// Owned continuation for the oracle-aligned bottom-up merge sort.
#[derive(Clone, Debug)]
pub(crate) struct TableSortState {
    pub table: GcRef<Table>,
    pub comparator: Option<Value>,
    pub less_key: Option<GcRef<GcString>>,
    pub nil_metatable: Option<GcRef<Table>>,
    pub boolean_metatable: Option<GcRef<Table>>,
    pub number_metatable: Option<GcRef<Table>>,
    pub values: Vec<Value>,
    pub scratch: Vec<Value>,
    pub stage: TableSortStage,
    pub pending: Option<TableSortPending>,
}

impl TableSortState {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&Value::Table(self.table));
        if let Some(comparator) = &self.comparator {
            gc.mark_value(comparator);
        }
        if let Some(less_key) = self.less_key {
            gc.mark_value(&Value::String(less_key));
        }
        for metatable in [
            self.nil_metatable,
            self.boolean_metatable,
            self.number_metatable,
        ]
        .into_iter()
        .flatten()
        {
            gc.mark_value(&Value::Table(metatable));
        }
        for value in self.values.iter().chain(&self.scratch) {
            gc.mark_value(value);
        }
    }
}

/// Exact phase of the transactional table-sort working copy.
#[derive(Clone, Debug)]
pub(crate) enum TableSortStage {
    ValidateComparator {
        index: usize,
    },
    Merge {
        width: usize,
        left: usize,
        middle: usize,
        right: usize,
        first: usize,
        second: usize,
        output: usize,
    },
    Complete,
}

/// Comparator result expected by the next table-sort continuation turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableSortPending {
    ValidatePrimary,
    ValidateReverse,
    MergePrimary { custom: bool },
    MergeReverse,
}

/// Owned stdlib callback transferred to the Runtime scheduler.
#[derive(Clone, Debug)]
pub(crate) struct CallbackHelperRequest {
    pub id: NativeRequestId,
    pub kind: CallbackHelperKind,
    pub function: Value,
    pub args: Vec<Value>,
    pub deferred: Option<DeferredNativeCall>,
}

impl CallbackHelperRequest {
    pub(crate) fn seed_roots(&self, gc: &mut GarbageCollector) {
        gc.mark_value(&self.function);
        for value in &self.args {
            gc.mark_value(value);
        }
        if let Some(deferred) = &self.deferred {
            if let Some(proto) = deferred.caller_proto {
                gc.mark_registered(proto);
            }
            deferred.snapshot.seed_roots(gc);
        }
        if let CallbackHelperKind::TableSort { state, .. } = &self.kind {
            state.seed_roots(gc);
        }
        if let CallbackHelperKind::TableForeach { state } = &self.kind {
            state.seed_roots(gc);
        }
        if let CallbackHelperKind::PackageRequire { loaded, module_key } = &self.kind {
            gc.mark_value(&Value::Table(*loaded));
            gc.mark_value(module_key);
        }
        if let CallbackHelperKind::PackageModuleOptions { state, .. } = &self.kind {
            state.seed_roots(gc);
        }
        if let CallbackHelperKind::Gsub { state, .. } = &self.kind {
            state.seed_roots(gc);
        }
        if let CallbackHelperKind::DebugHook { continuation, .. } = &self.kind {
            continuation.seed_roots(gc);
        }
        if let CallbackHelperKind::VmMetamethod { continuation, .. } = &self.kind {
            continuation.seed_roots(gc);
        }
    }
}

/// One sealed Runtime-native mailbox payload.
#[derive(Clone, Debug)]
pub(crate) enum RuntimeRequest {
    Resume(ResumeRequest),
    FullCollection(FullCollectionRequest),
    DebugUpvalue(Box<DebugUpvalueRequest>),
    ProtectedCall(Box<ProtectedCallRequest>),
    CallbackHelper(Box<CallbackHelperRequest>),
}

impl RuntimeRequest {
    pub(crate) fn id(&self) -> NativeRequestId {
        match self {
            Self::Resume(request) => request.id,
            Self::FullCollection(request) => request.id,
            Self::DebugUpvalue(request) => request.id,
            Self::ProtectedCall(request) => request.id,
            Self::CallbackHelper(request) => request.id,
        }
    }

    pub(crate) fn deferred(&self) -> Option<&DeferredNativeCall> {
        match self {
            Self::Resume(request) => request.deferred.as_ref(),
            Self::FullCollection(request) => request.deferred.as_ref(),
            Self::DebugUpvalue(request) => request.deferred.as_ref(),
            Self::ProtectedCall(request) => request.deferred.as_ref(),
            Self::CallbackHelper(request) => request.deferred.as_ref(),
        }
    }

    pub(crate) fn deferred_mut(&mut self) -> &mut Option<DeferredNativeCall> {
        match self {
            Self::Resume(request) => &mut request.deferred,
            Self::FullCollection(request) => &mut request.deferred,
            Self::DebugUpvalue(request) => &mut request.deferred,
            Self::ProtectedCall(request) => &mut request.deferred,
            Self::CallbackHelper(request) => &mut request.deferred,
        }
    }

    pub(crate) fn is_ready_for_runtime(&self) -> bool {
        self.deferred().is_some()
            || matches!(
                self,
                Self::CallbackHelper(request)
                    if matches!(
                        &request.kind,
                        CallbackHelperKind::DebugHook { .. }
                            | CallbackHelperKind::VmMetamethod { .. }
                    )
            )
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

//! Unique owner for one Lua runtime.
//!
//! `Runtime` keeps one [`Heap`], the generational state arena, and runtime
//! services in a pinned storage allocation. The Heap couples collector
//! accounting and the canonical string pool under one non-reused identity. A
//! generational arena owns the main `LuaState` and every coroutine state.
//! LuaState stores no collector or string-pool backpointer: each state
//! turn installs a dynamically scoped service context. Callers can borrow the
//! three execution parts together through `Runtime::parts_mut`; the returned
//! guard records Runtime execution and the main-state arena borrow. Explicit
//! component borrows do not install dynamic Heap access; callers enter a
//! synchronous context through the guard when native services are needed.
//!
//! The first coroutine-trampoline substrate additionally provides a
//! Runtime-owned turn driver. It confines one state borrow to one callback,
//! releases that guard before resolving a requested next handle, and retains
//! only owned `StateHandle`/result values between turns. Coroutine status,
//! result delivery, and VM continuation semantics remain a later integration.
//!
//! M1.8 adds explicit deterministic reclamation for shutdown. `Runtime::close`
//! drains remaining Lua `__gc` callbacks with error isolation, drains
//! coroutine states and the main state, closes collector-validated open
//! Upvalues while their stacks are alive, clears Runtime roots, and calls
//! the collector's non-collecting `destroy_all` path. That path destroys
//! non-fixed Threads first, other non-fixed objects second, and fixed objects
//! last while a live StringPool removes every String index. Owner-thread
//! `Drop` uses the same implementation.
//!
//! Runtime services may register weak object identities with static drain
//! hooks. Close runs those hooks after Lua finalizers and before states or the
//! heap are destroyed, isolating individual errors and panics. The shared
//! managed-payload allocator ledger reports live/peak/cumulative bytes and
//! reaches zero after deterministic close. Live execution supports explicit
//! full/incremental collection plus allocation-triggered collection at
//! Runtime-owned instruction boundaries.

use std::cell::{RefCell, UnsafeCell};
use std::collections::HashSet;
use std::io::Write;
use std::marker::{PhantomData, PhantomPinned};
use std::num::NonZeroU64;
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
use std::thread::{self, ThreadId};

use lua_core::allocator::{
    AllocationAccount, AllocationLedger, AllocationSite, AllocatorStats, ManagedAllocationError,
};
use lua_core::function::Function;
use lua_core::gc::collector::{GarbageCollector, GcRefValidationError, IncrementalPhase};
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::object_id::ObjectId;
use lua_core::gc::publication::{PublicationTxn, Rooted};
use lua_core::gc_string::GcString;
use lua_core::heap::Heap;
use lua_core::metatable::METAMETHOD_NAMES;
use lua_core::proto::Proto;
use lua_core::state_handle::StateHandleIssuer;
pub use lua_core::state_handle::{RuntimeId, RuntimeIdExhausted, StateHandle};
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::thread::{CoroutineStatus, Thread};
use lua_core::upvalue::Upvalue;
use lua_core::userdata::Userdata;
use lua_core::value::Value;
use thiserror::Error;

use crate::execute::{
    ExecResult, GsubAdvance, RuntimeError, TableForeachAdvance, TableSortAdvance, VmExit,
    abort_finalizer_callback, abort_runtime_callback_turn, advance_gsub_state,
    advance_table_foreach_state, advance_table_sort_state, call_value_with_results,
    commit_table_sort, definite_load_syntax_error, dispatch_debug_line_hook,
    dispatch_started_lua_call_hook, execute_proto, finish_deferred_gsub_result,
    finish_deferred_load_error_message, finish_deferred_load_source, finish_deferred_native_call,
    finish_deferred_native_values, finish_deferred_protected_values,
    finish_deferred_runtime_result, finish_lua_return_after_debug_hook,
    finish_native_call_after_debug_hook, finish_package_require_callback,
    finish_protected_callback_turn, finish_runtime_callback_turn, finish_vm_metamethod_callback,
    publish_debug_hook_event, resume_after_deferred_native_call, resume_finalizer_callback,
    resume_lua_call_after_debug_hook, resume_lua_instruction_after_debug_hook, resume_lua_thread,
    resume_native_call_after_debug_hook, resume_runtime_callback, start_finalizer_callback,
    start_lua_call_at_stack, start_runtime_callback_target, unwind_top_level_lua_error,
};
use crate::native::{
    CallbackHelperKind, CallbackHelperRequest, DebugHookContinuation, DebugUpvalueOperation,
    DebugUpvalueRequest, DeferredNativeCall, FullCollectionRequest, ProtectedCallKind,
    ProtectedCallRequest, ResumeRequest, ResumeResponse, RuntimeRequest, UpvalueAccessOperation,
    UpvalueAccessRequest,
};
use crate::state::lua_state::LuaStateShutdownReport;
use crate::state::{LuaState, ThreadStatus, enter_vm_context, with_vm_context, with_vm_context_gc};
use crate::trace::{
    InstructionTrace, InstructionTraceError, InstructionTraceMode, InstructionTraceSummary,
    SharedInstructionTrace, discard_all_instruction_diffs, emit_coroutine_resume_trace,
    emit_runtime_error_trace, end_trace_callback_barrier, record_trace_nil_range,
    retire_trace_state,
};

mod full_collection;
mod incremental_collection;
mod root_trace;
use full_collection::collect_full_stw_at_safe_point;
use incremental_collection::collect_incremental_step_at_safe_point;
use root_trace::{IncrementalRootTrace, RuntimeRootSet};
pub use root_trace::{
    MarkOnlyReport, RootEdgeCount, RuntimeRootKind, StateTraceFailure, UnresolvedObjectEdge,
    UnsafeTraceGap, UnsafeTraceGapKind,
};

/// Lifecycle phase of a `Runtime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePhase {
    /// The runtime accepts execution borrows.
    Running,
    /// Logical close has started; no new execution is accepted.
    Closing,
    /// The main state has been destroyed.
    Closed,
}

/// Runtime scheduler boundary used by one-shot durability fault injection.
///
/// This is a hidden test surface rather than a Lua-visible execution policy.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeFaultSite {
    /// A Runtime-native request was published, sealed, and removed from the
    /// requester mailbox, but has not yet been scheduled.
    SealedMailboxTake,
    /// An open-Upvalue transfer frame was rooted, but its owner state has not
    /// yet been resolved.
    UpvalueOwnerResolve,
    /// An owner response was rooted, but has not yet been delivered to the
    /// requester state.
    UpvalueResponseDelivery,
    /// A coroutine stopped while its activation links still need restoration.
    ActivationUnwind,
    /// Shutdown validation passed, but no finalizer or ownership mutation has
    /// started.
    ShutdownPreflight,
}

/// Observable scheduler invariants from the most recent execution session.
///
/// Current depths must return to zero after every success or failure. Peak
/// values make deep-chain tests independent of host stack layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeActivationStats {
    /// Runtime-owned state turns executed in the session.
    pub executed_turns: usize,
    /// Largest number of nested coroutine resume/wrap frames.
    pub peak_resume_depth: usize,
    /// Largest number of ordinary open-Upvalue transfer frames.
    pub peak_upvalue_transfer_depth: usize,
    /// Largest number of debug-library open-Upvalue transfer frames.
    pub peak_debug_upvalue_transfer_depth: usize,
    /// Largest number of nested Runtime-owned `pcall`/`xpcall` helpers.
    pub peak_protected_call_depth: usize,
    /// Largest number of nested ordinary stdlib callback helpers.
    pub peak_callback_helper_depth: usize,
    /// Largest combined activation/root-transfer buffer depth.
    pub peak_rooted_activation_depth: usize,
    /// Largest number of StateArena slots borrowed at the same instant.
    pub peak_borrowed_state_slots: usize,
    /// Coroutine frames still retained after the session.
    pub current_resume_depth: usize,
    /// Ordinary open-Upvalue transfers still retained after the session.
    pub current_upvalue_transfer_depth: usize,
    /// Debug open-Upvalue transfers still retained after the session.
    pub current_debug_upvalue_transfer_depth: usize,
    /// Protected helper frames still retained after the session.
    pub current_protected_call_depth: usize,
    /// Ordinary stdlib callback frames still retained after the session.
    pub current_callback_helper_depth: usize,
    /// GC/finalizer frames still retained after the session.
    pub current_gc_depth: usize,
}

impl RuntimeActivationStats {
    /// Total Runtime-owned activation and transfer frames still retained.
    pub fn current_rooted_activation_depth(self) -> usize {
        self.current_resume_depth
            .saturating_add(self.current_upvalue_transfer_depth)
            .saturating_add(self.current_debug_upvalue_transfer_depth)
            .saturating_add(self.current_protected_call_depth)
            .saturating_add(self.current_callback_helper_depth)
            .saturating_add(self.current_gc_depth)
    }
}

/// Failure to access or close a runtime.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeAccessError {
    /// The operation was attempted from a thread other than the creator.
    #[error("runtime {runtime_id:?} belongs to thread {owner:?}, not {current:?}")]
    WrongThread {
        /// Runtime being accessed.
        runtime_id: RuntimeId,
        /// Thread that created the runtime.
        owner: ThreadId,
        /// Thread attempting the operation.
        current: ThreadId,
    },
    /// Execution parts were requested outside the running phase.
    #[error("runtime {runtime_id:?} is in phase {phase:?}")]
    NotRunning {
        /// Runtime being accessed.
        runtime_id: RuntimeId,
        /// Current lifecycle phase.
        phase: RuntimePhase,
    },
    /// An exclusive Runtime operation was requested during active execution.
    #[error("runtime {runtime_id:?} has {count} active execution(s)")]
    ActiveExecutions {
        /// Runtime being closed.
        runtime_id: RuntimeId,
        /// Number of active execution guards.
        count: usize,
    },
    /// A running runtime lost its main-state owner, violating its invariant.
    #[error("runtime {runtime_id:?} has no main state while running")]
    MainStateUnavailable {
        /// Runtime whose invariant was violated.
        runtime_id: RuntimeId,
    },
    /// The state arena rejected an internal ownership transition.
    #[error("runtime {runtime_id:?} state arena rejected operation: {source}")]
    StateArena {
        /// Runtime being accessed.
        runtime_id: RuntimeId,
        /// Arena validation failure.
        source: StateResolveError,
    },
    /// A durability-test fault stopped the operation at its named boundary.
    #[error("runtime {runtime_id:?} injected a fault at {site:?}")]
    InjectedFault {
        /// Runtime whose operation was interrupted.
        runtime_id: RuntimeId,
        /// Boundary selected by the one-shot fault plan.
        site: RuntimeFaultSite,
    },
}

/// Failure while executing through the Runtime-owned state scheduler.
#[derive(Debug, Error)]
pub enum RuntimeExecutionError {
    /// Runtime ownership, phase, or state-arena validation failed.
    #[error(transparent)]
    Access(#[from] RuntimeAccessError),
    /// Lua VM execution failed.
    #[error(transparent)]
    Vm(#[from] RuntimeError),
    /// The sealed native request protocol violated an internal invariant.
    #[error("runtime-native protocol error: {0}")]
    Protocol(String),
}

/// Result produced by one Runtime-owned state execution turn.
///
/// This is the ownership substrate for the future coroutine trampoline. A
/// `Switch` ends the current state borrow before the Runtime acquires the next
/// handle; `Complete` ends the driver session.
#[allow(
    dead_code,
    reason = "M1 Runtime turn substrate is integrated before coroutine dispatch"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeTurn<T> {
    /// Release the current state and acquire `StateHandle` for the next turn.
    Switch(StateHandle),
    /// Release the current state and return the completed value.
    Complete(T),
}

#[derive(Clone, Debug)]
struct NativeActivationFrame {
    caller: StateHandle,
    caller_thread: Option<GcRef<Thread>>,
    previous_caller_status: Option<CoroutineStatus>,
    previous_target_status: CoroutineStatus,
    previous_target_caller: Option<GcRef<Thread>>,
    request: ResumeRequest,
    pending_response: Option<ResumeResponse>,
    replay_dead_ancestor: bool,
}

#[derive(Debug, Default)]
struct NativeActivationStack {
    frames: Vec<NativeActivationFrame>,
    upvalue_transfers: Vec<UpvalueTransferFrame>,
    debug_upvalue_transfers: Vec<DebugUpvalueTransferFrame>,
    protected_calls: Vec<ProtectedCallActivationFrame>,
    callback_helpers: Vec<CallbackHelperActivationFrame>,
    gc_frames: Vec<GcActivationFrame>,
    next_callback_sequence: u64,
    stats: NativeActivationCounters,
}

impl NativeActivationStack {
    fn begin_session(&mut self) {
        self.stats = NativeActivationCounters::default();
        self.next_callback_sequence = 1;
    }

    fn allocate_callback_sequence(&mut self) -> Result<u64, RuntimeExecutionError> {
        let sequence = self.next_callback_sequence;
        self.next_callback_sequence =
            self.next_callback_sequence.checked_add(1).ok_or_else(|| {
                RuntimeExecutionError::Protocol(
                    "Runtime callback activation sequence exhausted".to_string(),
                )
            })?;
        Ok(sequence)
    }

    fn observe_turn(&mut self) {
        self.stats.executed_turns = self.stats.executed_turns.saturating_add(1);
        self.stats.peak_resume_depth = self.stats.peak_resume_depth.max(self.frames.len());
        self.stats.peak_upvalue_transfer_depth = self
            .stats
            .peak_upvalue_transfer_depth
            .max(self.upvalue_transfers.len());
        self.stats.peak_debug_upvalue_transfer_depth = self
            .stats
            .peak_debug_upvalue_transfer_depth
            .max(self.debug_upvalue_transfers.len());
        self.stats.peak_protected_call_depth = self
            .stats
            .peak_protected_call_depth
            .max(self.protected_calls.len());
        self.stats.peak_callback_helper_depth = self
            .stats
            .peak_callback_helper_depth
            .max(self.callback_helpers.len());
        let rooted_depth = self
            .frames
            .len()
            .saturating_add(self.upvalue_transfers.len())
            .saturating_add(self.debug_upvalue_transfers.len())
            .saturating_add(self.protected_calls.len())
            .saturating_add(self.callback_helpers.len())
            .saturating_add(self.gc_frames.len());
        self.stats.peak_rooted_activation_depth =
            self.stats.peak_rooted_activation_depth.max(rooted_depth);
    }

    fn snapshot(&self, peak_borrowed_state_slots: usize) -> RuntimeActivationStats {
        RuntimeActivationStats {
            executed_turns: self.stats.executed_turns,
            peak_resume_depth: self.stats.peak_resume_depth,
            peak_upvalue_transfer_depth: self.stats.peak_upvalue_transfer_depth,
            peak_debug_upvalue_transfer_depth: self.stats.peak_debug_upvalue_transfer_depth,
            peak_protected_call_depth: self.stats.peak_protected_call_depth,
            peak_callback_helper_depth: self.stats.peak_callback_helper_depth,
            peak_rooted_activation_depth: self.stats.peak_rooted_activation_depth,
            peak_borrowed_state_slots,
            current_resume_depth: self.frames.len(),
            current_upvalue_transfer_depth: self.upvalue_transfers.len(),
            current_debug_upvalue_transfer_depth: self.debug_upvalue_transfers.len(),
            current_protected_call_depth: self.protected_calls.len(),
            current_callback_helper_depth: self.callback_helpers.len(),
            current_gc_depth: self.gc_frames.len(),
        }
    }

    fn seed_roots(&self, gc: &mut GarbageCollector) {
        for frame in &self.frames {
            frame.request.seed_roots(gc);
            if let Some(caller) = frame.caller_thread {
                gc.mark_value(&Value::Thread(caller));
            }
            if let Some(response) = &frame.pending_response {
                response.seed_roots(gc);
            }
        }
        for transfer in &self.upvalue_transfers {
            transfer.request.seed_roots(gc);
            if let Some(response) = &transfer.response {
                response.seed_roots(gc);
            }
        }
        for transfer in &self.debug_upvalue_transfers {
            transfer.request.seed_roots(gc);
            if let Some(response) = &transfer.response {
                response.seed_roots(gc);
            }
        }
        for frame in &self.protected_calls {
            frame.seed_roots(gc);
        }
        for frame in &self.callback_helpers {
            frame.seed_roots(gc);
        }
        for frame in &self.gc_frames {
            frame.seed_roots(gc);
            if let Some((userdata, callback)) = &frame.current_finalizer {
                gc.mark_value(&Value::Userdata(*userdata));
                gc.mark_value(callback);
            }
            if let Some(response) = &frame.response {
                response.seed_roots(gc);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeActivationCounters {
    executed_turns: usize,
    peak_resume_depth: usize,
    peak_upvalue_transfer_depth: usize,
    peak_debug_upvalue_transfer_depth: usize,
    peak_protected_call_depth: usize,
    peak_callback_helper_depth: usize,
    peak_rooted_activation_depth: usize,
}

#[derive(Clone, Copy, Debug)]
struct RuntimeFaultInjection {
    site: RuntimeFaultSite,
    successful_matching_checkpoints: usize,
}

impl RuntimeFaultInjection {
    fn checkpoint(plan: &mut Option<Self>, site: RuntimeFaultSite) -> bool {
        let Some(active) = plan.as_mut() else {
            return false;
        };
        if active.site != site {
            return false;
        }
        if active.successful_matching_checkpoints != 0 {
            active.successful_matching_checkpoints -= 1;
            return false;
        }
        *plan = None;
        true
    }
}

#[derive(Clone, Debug)]
enum GcActivationOrigin {
    Explicit(Box<FullCollectionRequest>),
    Automatic { stop_ci: usize, saved_top: usize },
}

#[derive(Clone, Debug)]
struct GcActivationFrame {
    requester: StateHandle,
    origin: GcActivationOrigin,
    resume: NativeDriverRole,
    draining_finalizers: bool,
    current_finalizer: Option<(GcRef<Userdata>, Value)>,
    response: Option<CollectionResponse>,
}

impl GcActivationFrame {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        if let GcActivationOrigin::Explicit(request) = &self.origin {
            request.seed_roots(gc);
        }
    }

    fn explicit_request(&self) -> Option<&FullCollectionRequest> {
        match &self.origin {
            GcActivationOrigin::Explicit(request) => Some(request),
            GcActivationOrigin::Automatic { .. } => None,
        }
    }

    fn stop_ci(&self) -> Option<usize> {
        match &self.origin {
            GcActivationOrigin::Explicit(request) => request
                .deferred
                .as_ref()
                .map(|deferred| deferred.saved_ci + 1),
            GcActivationOrigin::Automatic { stop_ci, .. } => Some(*stop_ci),
        }
    }

    fn saved_top(&self) -> Option<usize> {
        match &self.origin {
            GcActivationOrigin::Explicit(request) => request
                .deferred
                .as_ref()
                .map(|deferred| deferred.snapshot.top()),
            GcActivationOrigin::Automatic { saved_top, .. } => Some(*saved_top),
        }
    }

    fn automatic_resume(&self) -> Option<NativeDriverRole> {
        match self.origin {
            GcActivationOrigin::Explicit(_) => None,
            GcActivationOrigin::Automatic { .. } => Some(self.resume),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtectedCallPhase {
    Target,
    Handler,
}

#[derive(Clone, Debug)]
struct ProtectedCallActivationFrame {
    requester: StateHandle,
    request: ProtectedCallRequest,
    phase: ProtectedCallPhase,
    role: NativeDriverRole,
    parent_role: NativeDriverRole,
    previous_allow_yield: u16,
    pending_response: Option<ResumeResponse>,
    sequence: u64,
}

impl ProtectedCallActivationFrame {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        self.request.seed_roots(gc);
        if let Some(response) = &self.pending_response {
            response.seed_roots(gc);
        }
    }
}

#[derive(Clone, Debug)]
struct CallbackHelperActivationFrame {
    requester: StateHandle,
    request: CallbackHelperRequest,
    role: NativeDriverRole,
    parent_role: NativeDriverRole,
    pending_response: Option<ResumeResponse>,
    sequence: u64,
}

impl CallbackHelperActivationFrame {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        self.request.seed_roots(gc);
        if let Some(response) = &self.pending_response {
            response.seed_roots(gc);
        }
    }
}

#[derive(Clone, Debug)]
struct UpvalueTransferFrame {
    request: UpvalueAccessRequest,
    response: Option<UpvalueAccessResponse>,
}

#[derive(Clone, Debug)]
struct DebugUpvalueTransferFrame {
    request: DebugUpvalueRequest,
    response: Option<DebugUpvalueResponse>,
}

#[derive(Clone, Debug)]
enum UpvalueAccessResponse {
    Read(Value),
    Written,
}

impl UpvalueAccessResponse {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        if let Self::Read(value) = self {
            gc.mark_value(value);
        }
    }
}

#[derive(Clone, Debug)]
enum DebugUpvalueResponse {
    Success(Vec<Value>),
    ErrorMessage(String),
}

impl DebugUpvalueResponse {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        if let Self::Success(values) = self {
            for value in values {
                gc.mark_value(value);
            }
        }
    }
}

enum NativeDriverAction {
    StartProto(GcRef<Proto>),
    StartTarget {
        request: ResumeRequest,
        previous_status: CoroutineStatus,
        normal_deferred: Option<DeferredNativeCall>,
    },
    Deliver {
        frame: NativeActivationFrame,
        response: ResumeResponse,
        replay_dead_ancestor: bool,
        resume: NativeDriverRole,
    },
    StartProtectedCall {
        function: Value,
        args: Vec<Value>,
        role: NativeDriverRole,
    },
    FinishProtectedCall {
        frame: ProtectedCallActivationFrame,
        response: ResumeResponse,
    },
    StartCallbackHelper {
        function: Value,
        args: Vec<Value>,
        wanted_results: Option<usize>,
        role: NativeDriverRole,
    },
    FinishCallbackHelper {
        frame: CallbackHelperActivationFrame,
        response: ResumeResponse,
    },
    DeliverUpvalue {
        frame: UpvalueTransferFrame,
        resume: NativeDriverRole,
    },
    DeliverDebugUpvalue {
        frame: DebugUpvalueTransferFrame,
        resume: NativeDriverRole,
    },
    DeliverCollection {
        request: FullCollectionRequest,
        response: CollectionResponse,
        resume: NativeDriverRole,
    },
    ResumeAutomaticGc {
        resume: NativeDriverRole,
    },
    StartFinalizer {
        callback: Value,
        userdata: GcRef<Userdata>,
        stop_ci: usize,
    },
    FailFinalizer {
        message: String,
        stop_ci: usize,
        saved_top: usize,
    },
}

fn callback_helper_wanted_results(kind: &CallbackHelperKind) -> Option<usize> {
    match kind {
        CallbackHelperKind::DoFile => None,
        CallbackHelperKind::PackageModuleOptions { .. } => Some(0),
        CallbackHelperKind::ToString
        | CallbackHelperKind::LoadReader { .. }
        | CallbackHelperKind::TableSort { .. }
        | CallbackHelperKind::TableForeach { .. }
        | CallbackHelperKind::PackageRequire { .. }
        | CallbackHelperKind::Gsub { .. }
        | CallbackHelperKind::DebugHook { .. }
        | CallbackHelperKind::VmMetamethod { .. } => Some(1),
    }
}

#[derive(Clone, Debug)]
enum CollectionResponse {
    Success(Vec<Value>),
    ErrorMessage(String),
    ErrorValue(Value),
}

impl CollectionResponse {
    fn seed_roots(&self, gc: &mut GarbageCollector) {
        match self {
            Self::Success(values) => {
                for value in values {
                    gc.mark_value(value);
                }
            }
            Self::ErrorValue(error) => gc.mark_value(error),
            Self::ErrorMessage(_) => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeDriverRole {
    Program,
    Finalizer {
        stop_ci: usize,
    },
    ProtectedCall {
        stop_ci: usize,
        call_pos: usize,
        saved_top: usize,
    },
    CallbackHelper {
        stop_ci: usize,
        call_pos: usize,
        saved_top: usize,
    },
}

impl NativeDriverAction {
    fn role(&self) -> NativeDriverRole {
        match self {
            Self::StartFinalizer { stop_ci, .. } | Self::FailFinalizer { stop_ci, .. } => {
                NativeDriverRole::Finalizer { stop_ci: *stop_ci }
            }
            Self::StartProtectedCall { role, .. } => *role,
            Self::FinishProtectedCall { frame, .. } => frame.parent_role,
            Self::StartCallbackHelper { role, .. } => *role,
            Self::FinishCallbackHelper { frame, .. } => frame.parent_role,
            Self::Deliver { resume, .. }
            | Self::DeliverUpvalue { resume, .. }
            | Self::DeliverDebugUpvalue { resume, .. }
            | Self::ResumeAutomaticGc { resume } => *resume,
            Self::DeliverCollection { resume, .. } => *resume,
            _ => NativeDriverRole::Program,
        }
    }
}

enum NativeTurnEvent {
    Request {
        request: Box<RuntimeRequest>,
        caller_thread: Option<GcRef<Thread>>,
    },
    UpvalueAccess(Box<UpvalueAccessRequest>),
    AutomaticGc {
        stop_ci: usize,
        saved_top: usize,
    },
    Main(Result<ExecResult, RuntimeError>),
    Finalizer(Result<(), RuntimeError>),
    ProtectedCall(ResumeResponse),
    CallbackHelper(ResumeResponse),
    Coroutine {
        thread: GcRef<Thread>,
        response: ResumeResponse,
    },
}

/// Result of explicit Runtime teardown.
///
/// M1.8 reclaims Rust-owned states and collector allocations. Close drains
/// Lua-visible `__gc` callbacks with error isolation before destroying the
/// heap; Runtime service hooks drain external resources between finalizers and
/// state destruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeCloseReport {
    /// Runtime that produced the report.
    pub runtime_id: RuntimeId,
    /// Whether this call observed a runtime that was already closed.
    pub already_closed: bool,
    /// Whether collector-managed heap reclamation remains deferred.
    pub heap_reclamation_deferred: bool,
    /// Objects still linked in the collector.
    pub remaining_objects: usize,
    /// Roots still recorded by the collector.
    pub remaining_roots: usize,
    /// Strings still indexed by the string pool.
    pub remaining_interned_strings: usize,
    /// Collector's current estimated live-byte counter.
    pub remaining_estimated_bytes: usize,
    /// Managed allocator payload bytes still live after close.
    pub remaining_allocator_live_bytes: usize,
    /// Highest managed allocator payload observed in this Runtime.
    pub allocator_peak_bytes: usize,
    /// Cumulative positive managed payload charges over the Runtime lifetime.
    pub allocator_total_allocated_bytes: usize,
    /// Coroutine states still retained after close.
    pub remaining_coroutine_states: usize,
    /// PendingState roots still retained after close.
    pub remaining_temporary_state_roots: usize,
    /// Exact-id PendingState root releases rejected by the arena.
    pub rejected_temporary_state_root_releases: usize,
    /// Transient gray/weak/finalizer/external collector queue entries.
    pub remaining_collector_queue_entries: usize,
    /// Coroutine states drained and generation-invalidated by the first close.
    pub drained_coroutine_states: usize,
    /// Open Upvalues safely closed while their owner state stacks were alive.
    pub closed_open_upvalues: usize,
    /// Invalid/cross-collector Upvalue list edges rejected before dereference.
    pub rejected_open_upvalue_edges: usize,
    /// Open Upvalues whose checked owner handle did not match the state.
    pub open_upvalue_owner_mismatches: usize,
    /// Open Upvalues whose stack slot was outside the active initialized view.
    pub open_upvalue_stack_values_missing: usize,
    /// Concrete collector allocations destroyed by the first close.
    pub destroyed_objects: usize,
    /// Non-fixed Thread allocations destroyed in the first GC teardown pass.
    pub destroyed_threads: usize,
    /// Fixed allocations destroyed in the final GC teardown pass.
    pub destroyed_fixed_objects: usize,
    /// Pending-finalizer queue entries discarded after the close drain.
    pub pending_lua_finalizers_discarded: usize,
    /// Lua `__gc` callbacks attempted during the close drain.
    pub lua_finalizers_attempted: usize,
    /// Close-time Lua `__gc` errors isolated while continuing the drain.
    pub lua_finalizer_errors_ignored: usize,
    /// Runtime resource drain hooks attempted during the first close.
    pub runtime_resources_attempted: usize,
    /// Runtime resources explicitly closed by their service hook.
    pub runtime_resources_drained: usize,
    /// Registered resources already closed before their service hook ran.
    pub runtime_resources_already_closed: usize,
    /// Weak resource identities no longer present in the collector.
    pub runtime_resources_stale: usize,
    /// Resource hook failures or panics isolated while continuing shutdown.
    pub runtime_resource_drain_errors: usize,
    /// Whether close-time Lua `__gc` callback delivery remains unavailable.
    pub lua_gc_callback_debt: bool,
    /// Capability debt: Runtime resource drain hooks are unavailable.
    pub io_resource_drain_debt: bool,
}

/// Result returned by one Runtime-owned external-resource drain hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeResourceDrainOutcome {
    /// The hook closed and released the external resource.
    Drained,
    /// Lua code or a finalizer had already closed the resource.
    AlreadyClosed,
    /// The weak object identity was already reclaimed by the collector.
    Stale,
    /// The resource remained open because its close operation failed.
    Failed,
}

/// Static hook used to drain one weakly registered Runtime resource.
pub type RuntimeResourceDrainHook =
    fn(&mut LuaState, &mut GarbageCollector, ObjectId) -> RuntimeResourceDrainOutcome;

#[derive(Clone, Copy, Debug)]
struct RuntimeResourceEntry {
    object_id: ObjectId,
    drain: RuntimeResourceDrainHook,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RuntimeResourceDrainReport {
    attempted: usize,
    drained: usize,
    already_closed: usize,
    stale: usize,
    errors: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RuntimeShutdownSummary {
    drained_coroutine_states: usize,
    state_shutdown: LuaStateShutdownReport,
    destroyed_objects: usize,
    destroyed_threads: usize,
    destroyed_fixed_objects: usize,
    pending_lua_finalizers_discarded: usize,
    lua_finalizers_attempted: usize,
    lua_finalizer_errors_ignored: usize,
    resource_drain: RuntimeResourceDrainReport,
}

/// Failure to resolve a generational Lua state handle.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StateResolveError {
    /// A standalone LuaState has no Runtime arena.
    #[error("LuaState is not attached to a Runtime StateArena")]
    ArenaUnavailable,
    /// The handle belongs to another runtime.
    #[error("state handle belongs to runtime {actual:?}, expected {expected:?}")]
    ForeignRuntime {
        /// Runtime owning the arena.
        expected: RuntimeId,
        /// Runtime encoded by the handle.
        actual: RuntimeId,
    },
    /// The handle's slot does not exist.
    #[error("state slot {slot} does not exist")]
    InvalidSlot {
        /// Invalid slot index.
        slot: usize,
    },
    /// The slot has been reused since this handle was issued.
    #[error("state slot {slot} generation is {actual}, but handle requested {requested}")]
    StaleGeneration {
        /// Reused slot index.
        slot: usize,
        /// Generation encoded by the stale handle.
        requested: u64,
        /// Current slot generation.
        actual: u64,
    },
    /// The slot exhausted its generation namespace and can never be reused.
    #[error("state slot {slot} is permanently retired at generation {generation}")]
    RetiredSlot {
        /// Permanently retired slot index.
        slot: usize,
        /// Final generation issued by the slot.
        generation: u64,
    },
    /// A valid-generation slot currently contains no state.
    #[error("state slot {slot} generation {generation} is vacant")]
    Vacant {
        /// Vacant slot index.
        slot: usize,
        /// Current slot generation.
        generation: u64,
    },
    /// The requested state is already exclusively borrowed by nested execution.
    #[error("state slot {slot} generation {generation} is already borrowed")]
    AlreadyBorrowed {
        /// Borrowed slot index.
        slot: usize,
        /// Borrowed slot generation.
        generation: u64,
    },
    /// Nested arena resolution was attempted during a Runtime-owned turn for a
    /// target that is not another Runtime-owned coroutine. Coroutine debug and
    /// library inspection uses a separately tracked scoped borrow.
    #[error(
        "runtime turn already owns state slot {slot} generation {generation}; publish a switch instead"
    )]
    TurnBorrowActive {
        /// Slot owned by the active Runtime turn.
        slot: usize,
        /// Generation owned by the active Runtime turn.
        generation: u64,
    },
    /// Resolution attempted to recreate the caller's current mutable state.
    #[error("cannot resolve the currently mutably borrowed LuaState")]
    CurrentState,
    /// A handle expected to name the main state named another arena state.
    #[error("state slot {slot} is not the Runtime main state")]
    NotMainState {
        /// Non-main slot index.
        slot: usize,
    },
    /// Internal StateArena bookkeeping disagreed before a destructive action.
    #[error("state arena invariant violated: {reason}")]
    ArenaInvariant {
        /// Stable diagnostic describing the rejected invariant.
        reason: &'static str,
    },
    /// A checked StateArena allocation failed before publishing a slot.
    #[error(transparent)]
    Allocation(#[from] ManagedAllocationError),
}

/// Failure while publishing one Runtime-owned coroutine state.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PendingStateError {
    /// The arena rejected the pending state's checked handle.
    #[error(transparent)]
    State(#[from] StateResolveError),
    /// The collector rejected a Thread or Function publication edge.
    #[error(transparent)]
    Object(#[from] GcRefValidationError),
    /// The pending root identity no longer names this state.
    #[error("pending state root no longer names its arena slot")]
    RootMismatch,
    /// A pending state was bound to a different Thread.
    #[error("pending state is already bound to a different Thread")]
    ThreadMismatch,
    /// Publication was attempted before both State→Thread and Thread→State
    /// edges were installed.
    #[error("pending state has not completed its checked Thread binding")]
    ThreadNotBound,
    /// A wrapper publication did not retain the bound Thread through its
    /// closed Upvalue graph.
    #[error("published coroutine wrapper does not retain the pending Thread")]
    WrapperThreadMismatch,
    /// Stack publication targeted a different LuaState than the transaction
    /// that created this PendingState.
    #[error("pending state publication targeted a different LuaState")]
    PublisherMismatch,
}

struct StateSlot {
    generation: u64,
    state: Option<NonNull<LuaState>>,
    accounted_bytes: usize,
    is_main: bool,
    borrowed: bool,
    retired: bool,
    temporary_root_id: Option<u64>,
}

/// Runtime-owned generational arena for the main and coroutine Lua states.
///
/// Slots contain stable `NonNull<LuaState>` addresses rather than references.
/// All resolution validates runtime, slot, generation, occupancy, current-state
/// identity, and exclusive-borrow state. References are created only inside a
/// higher-ranked closure and therefore cannot escape the resolution scope.
pub struct StateArena {
    handle_issuer: StateHandleIssuer,
    slots: Vec<StateSlot>,
    free_slots: Vec<usize>,
    live_coroutine_states: usize,
    next_temporary_root_id: u64,
    rejected_temporary_root_releases: usize,
    turn_borrow: Option<StateHandle>,
    runtime_resources: Vec<RuntimeResourceEntry>,
    runtime_resource_drain_cursor: usize,
    allocation_account: AllocationAccount,
    #[cfg(test)]
    turn_borrow_events: Vec<TurnBorrowEvent>,
    peak_turn_borrowed_slots: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnBorrowEvent {
    Acquired(StateHandle),
    Released(StateHandle),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StateArenaDrainReport {
    drained_coroutine_states: usize,
    drained_main_states: usize,
    state_shutdown: LuaStateShutdownReport,
    #[cfg(test)]
    drained_state_handles: Vec<StateHandle>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StateArenaCollectionReport {
    swept_state_handles: Vec<StateHandle>,
    state_shutdown: LuaStateShutdownReport,
}

impl StateArena {
    fn new(handle_issuer: StateHandleIssuer, allocator: AllocationLedger) -> Self {
        Self {
            handle_issuer,
            slots: Vec::new(),
            free_slots: Vec::new(),
            live_coroutine_states: 0,
            next_temporary_root_id: 1,
            rejected_temporary_root_releases: 0,
            turn_borrow: None,
            runtime_resources: Vec::new(),
            runtime_resource_drain_cursor: 0,
            allocation_account: AllocationAccount::new(allocator),
            #[cfg(test)]
            turn_borrow_events: Vec::new(),
            peak_turn_borrowed_slots: 0,
        }
    }

    fn runtime_id(&self) -> RuntimeId {
        self.handle_issuer.runtime_id()
    }

    fn reserve_slot(&mut self) -> Result<StateHandle, ManagedAllocationError> {
        let slot = loop {
            match self.free_slots.pop() {
                Some(slot_index)
                    if self.slots.get(slot_index).is_some_and(|slot| {
                        !slot.retired
                            && slot.state.is_none()
                            && !slot.is_main
                            && !slot.borrowed
                            && slot.temporary_root_id.is_none()
                    }) =>
                {
                    break slot_index;
                }
                Some(_) => {
                    // Fail closed if an internal free-list entry is stale or
                    // duplicated: discard it instead of overwriting a live or
                    // permanently retired slot.
                }
                None => {
                    let requested_bytes = std::mem::size_of::<StateSlot>();
                    self.allocation_account
                        .allocation_checkpoint(AllocationSite::StateArenaSlot, requested_bytes)?;
                    self.slots.try_reserve(1).map_err(|source| {
                        ManagedAllocationError::capacity(
                            AllocationSite::StateArenaSlot,
                            requested_bytes,
                            source,
                        )
                    })?;
                    self.slots.push(StateSlot {
                        generation: 1,
                        state: None,
                        accounted_bytes: 0,
                        is_main: false,
                        borrowed: false,
                        retired: false,
                        temporary_root_id: None,
                    });
                    break self.slots.len() - 1;
                }
            }
        };
        Ok(self.issue_handle(slot))
    }

    fn issue_handle(&self, slot: usize) -> StateHandle {
        let generation = NonZeroU64::new(self.slots[slot].generation)
            .expect("StateArena generations are always non-zero");
        self.handle_issuer.issue(slot, generation)
    }

    #[cfg(test)]
    fn insert_owned(&mut self, state: Box<LuaState>) -> StateHandle {
        self.insert_owned_with_temporary_root(state, None, None, false)
            .expect("test-owned StateArena insertion succeeds")
    }

    fn insert_main_owned(
        &mut self,
        state: Box<LuaState>,
        owner_cell: NonNull<UnsafeCell<StateArena>>,
    ) -> Result<StateHandle, ManagedAllocationError> {
        self.insert_owned_with_temporary_root(state, None, Some(owner_cell), true)
    }

    fn insert_pending_owned(
        &mut self,
        state: Box<LuaState>,
        owner_cell: Option<NonNull<UnsafeCell<StateArena>>>,
    ) -> Result<(StateHandle, u64), StateResolveError> {
        if state.state_handle.is_some()
            || state.state_arena.is_some()
            || !state.open_upvalues.is_empty()
        {
            return Err(StateResolveError::ArenaInvariant {
                reason: "pending state must be detached and have no open Upvalues",
            });
        }
        let temporary_root_id = self.next_temporary_root_id;
        let next_temporary_root_id =
            temporary_root_id
                .checked_add(1)
                .ok_or(StateResolveError::ArenaInvariant {
                    reason: "temporary state-root identity space exhausted",
                })?;
        let handle = self.insert_owned_with_temporary_root(
            state,
            Some(temporary_root_id),
            owner_cell,
            false,
        )?;
        self.next_temporary_root_id = next_temporary_root_id;
        Ok((handle, temporary_root_id))
    }

    fn insert_owned_with_temporary_root(
        &mut self,
        mut state: Box<LuaState>,
        temporary_root_id: Option<u64>,
        owner_cell: Option<NonNull<UnsafeCell<StateArena>>>,
        is_main: bool,
    ) -> Result<StateHandle, ManagedAllocationError> {
        debug_assert!(!is_main || temporary_root_id.is_none());
        let next_live_coroutine_states = if is_main {
            self.live_coroutine_states
        } else {
            self.live_coroutine_states
                .checked_add(1)
                .expect("StateArena live coroutine-state count overflow")
        };
        let handle = self.reserve_slot()?;
        state.attach_runtime_state(handle, owner_cell);
        let accounted_bytes = state.allocator_payload_bytes();
        let state =
            NonNull::new(Box::into_raw(state)).expect("Box::into_raw never returns a null pointer");

        let slot = &mut self.slots[handle.slot()];
        slot.state = Some(state);
        slot.accounted_bytes = accounted_bytes;
        slot.is_main = is_main;
        slot.temporary_root_id = temporary_root_id;
        self.live_coroutine_states = next_live_coroutine_states;
        self.reconcile_allocation_account();
        Ok(handle)
    }

    fn validate_pending(
        &self,
        handle: StateHandle,
        temporary_root_id: u64,
    ) -> Result<(usize, NonNull<LuaState>), PendingStateError> {
        let (slot_index, state) = self.validate(handle)?;
        let slot = &self.slots[slot_index];
        if slot.is_main || slot.temporary_root_id != Some(temporary_root_id) {
            return Err(PendingStateError::RootMismatch);
        }
        Ok((slot_index, state))
    }

    fn bind_pending_thread(
        &mut self,
        handle: StateHandle,
        temporary_root_id: u64,
        thread: GcRef<Thread>,
    ) -> Result<(), PendingStateError> {
        let (_, mut state) = self.validate_pending(handle, temporary_root_id)?;
        // SAFETY: the pending root exclusively owns this unpublished child;
        // it is not borrowable through a Thread until the reverse edge below
        // is installed, and Runtime execution is single-threaded.
        let state = unsafe { state.as_mut() };
        if state
            .current_thread
            .is_some_and(|current| current != thread)
        {
            return Err(PendingStateError::ThreadMismatch);
        }
        state.current_thread = Some(thread);
        Ok(())
    }

    fn pending_thread(
        &self,
        handle: StateHandle,
        temporary_root_id: u64,
        thread_id: ObjectId,
    ) -> Result<GcRef<Thread>, PendingStateError> {
        let (_, state) = self.validate_pending(handle, temporary_root_id)?;
        // SAFETY: validation proved this is the occupied pending slot. This
        // immutable identity read cannot escape the PendingState operation.
        let state = unsafe { state.as_ref() };
        let thread = state
            .current_thread
            .ok_or(PendingStateError::ThreadNotBound)?;
        if thread.object_id() != thread_id {
            return Err(PendingStateError::ThreadMismatch);
        }
        Ok(thread)
    }

    fn commit_pending(
        &mut self,
        handle: StateHandle,
        temporary_root_id: u64,
    ) -> Result<(), PendingStateError> {
        let (slot_index, _) = self.validate_pending(handle, temporary_root_id)?;
        self.slots[slot_index].temporary_root_id = None;
        Ok(())
    }

    fn rollback_pending(
        &mut self,
        handle: StateHandle,
        temporary_root_id: u64,
    ) -> Result<(), PendingStateError> {
        let (slot_index, state) = self.validate_pending(handle, temporary_root_id)?;
        if self.slots[slot_index].borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: handle.generation(),
            }
            .into());
        }
        let next_live_coroutine_states = self
            .live_coroutine_states
            .checked_sub(1)
            .expect("pending coroutine slot requires a positive live count");
        self.vacate_slot(slot_index);
        self.live_coroutine_states = next_live_coroutine_states;
        // SAFETY: pending slots originate from one Box::into_raw and have just
        // been made vacant without ever transferring ownership elsewhere.
        unsafe {
            drop(Box::from_raw(state.as_ptr()));
        }
        Ok(())
    }

    fn temporary_state_roots(&self) -> Vec<StateHandle> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.temporary_root_id.is_some() && slot.state.is_some())
            .map(|(slot_index, _)| self.issue_handle(slot_index))
            .collect()
    }

    fn temporary_state_root_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.temporary_root_id.is_some())
            .count()
    }

    fn reserve_runtime_resources(
        &mut self,
        additional: usize,
    ) -> Result<(), ManagedAllocationError> {
        let available = self
            .runtime_resources
            .capacity()
            .saturating_sub(self.runtime_resources.len());
        let additional = additional.saturating_sub(available);
        if additional == 0 {
            return Ok(());
        }
        let requested_bytes =
            additional.saturating_mul(std::mem::size_of::<RuntimeResourceEntry>());
        self.allocation_account
            .allocation_checkpoint(AllocationSite::RuntimeResourceRegistry, requested_bytes)?;
        self.runtime_resources
            .try_reserve(additional)
            .map_err(|source| {
                ManagedAllocationError::capacity(
                    AllocationSite::RuntimeResourceRegistry,
                    requested_bytes,
                    source,
                )
            })?;
        self.reconcile_allocation_account();
        Ok(())
    }

    fn register_runtime_resource(
        &mut self,
        object_id: ObjectId,
        drain: RuntimeResourceDrainHook,
    ) -> Result<bool, StateResolveError> {
        if object_id.is_null() {
            return Err(StateResolveError::ArenaInvariant {
                reason: "runtime resource registration has a null object identity",
            });
        }
        if self
            .runtime_resources
            .iter()
            .any(|entry| entry.object_id == object_id)
        {
            return Ok(false);
        }
        if self.runtime_resources.len() == self.runtime_resources.capacity() {
            return Err(StateResolveError::ArenaInvariant {
                reason: "runtime resource registration capacity was not reserved",
            });
        }
        self.runtime_resources
            .push(RuntimeResourceEntry { object_id, drain });
        Ok(true)
    }

    fn next_runtime_resource(&mut self) -> Option<RuntimeResourceEntry> {
        let entry = self
            .runtime_resources
            .get(self.runtime_resource_drain_cursor)
            .copied()?;
        self.runtime_resource_drain_cursor = self
            .runtime_resource_drain_cursor
            .checked_add(1)
            .expect("runtime resource drain cursor overflow");
        Some(entry)
    }

    fn pending_runtime_resource_count(&self) -> usize {
        self.runtime_resources
            .len()
            .saturating_sub(self.runtime_resource_drain_cursor)
    }

    fn finish_runtime_resource_drain(&mut self) {
        let resources = std::mem::take(&mut self.runtime_resources);
        drop(resources);
        self.runtime_resource_drain_cursor = 0;
        self.reconcile_allocation_account();
    }

    fn validate_runtime_resource_drain_complete(&self) -> Result<(), StateResolveError> {
        if !self.runtime_resources.is_empty() || self.runtime_resource_drain_cursor != 0 {
            return Err(StateResolveError::ArenaInvariant {
                reason: "runtime resources remain registered after the close drain",
            });
        }
        Ok(())
    }

    fn validate(
        &self,
        handle: StateHandle,
    ) -> Result<(usize, NonNull<LuaState>), StateResolveError> {
        if handle.runtime_id() != self.runtime_id() {
            return Err(StateResolveError::ForeignRuntime {
                expected: self.runtime_id(),
                actual: handle.runtime_id(),
            });
        }
        let Some(slot) = self.slots.get(handle.slot()) else {
            return Err(StateResolveError::InvalidSlot {
                slot: handle.slot(),
            });
        };
        if slot.retired {
            return Err(StateResolveError::RetiredSlot {
                slot: handle.slot(),
                generation: slot.generation,
            });
        }
        if slot.generation != handle.generation() {
            return Err(StateResolveError::StaleGeneration {
                slot: handle.slot(),
                requested: handle.generation(),
                actual: slot.generation,
            });
        }
        let state = slot.state.ok_or(StateResolveError::Vacant {
            slot: handle.slot(),
            generation: handle.generation(),
        })?;
        Ok((handle.slot(), state))
    }

    /// Resolve a handle to a validated stable pointer without creating a
    /// reference or extending its lifetime.
    fn resolve(
        &self,
        handle: StateHandle,
        current_state: NonNull<LuaState>,
    ) -> Result<NonNull<LuaState>, StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        if state == current_state {
            return Err(StateResolveError::CurrentState);
        }
        let slot = &self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        Ok(state)
    }

    /// Resolve a state for an immutable mark-only snapshot.
    ///
    /// Runtime calls this only while it has exclusive ownership access and no
    /// execution guard is active. Unlike execution resolution, the main state
    /// is a valid target; any borrowed slot is rejected deterministically.
    fn resolve_for_trace(
        &self,
        handle: StateHandle,
    ) -> Result<NonNull<LuaState>, StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        let slot = &self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        Ok(state)
    }

    fn main_state_ref(&self, handle: StateHandle) -> Result<&LuaState, StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        let slot = &self.slots[slot_index];
        if !slot.is_main {
            return Err(StateResolveError::NotMainState { slot: slot_index });
        }
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        // SAFETY: every occupied slot is owned by this arena. The returned
        // reference is tied to `&self`, which prevents safe Runtime mutation or
        // arena teardown for its lifetime, and the slot is not mutably borrowed.
        Ok(unsafe { state.as_ref() })
    }

    fn validate_main(&self, handle: StateHandle) -> Result<NonNull<LuaState>, StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        let slot = &self.slots[slot_index];
        if !slot.is_main {
            return Err(StateResolveError::NotMainState { slot: slot_index });
        }
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        Ok(state)
    }

    fn with_state_mut<R>(
        mut arena: NonNull<Self>,
        handle: StateHandle,
        current_state: NonNull<LuaState>,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        let state = {
            // SAFETY: callers pass the pinned Runtime-owned arena. This
            // mutable borrow ends before the target-state closure starts, so
            // nested state resolution cannot overlap an arena `&mut` borrow.
            let arena_ref = unsafe { arena.as_mut() };
            let state = arena_ref.resolve(handle, current_state)?;
            if let Some(active_handle) = arena_ref.turn_borrow {
                let (_, active_state) = arena_ref.validate(active_handle)?;
                // Execution switches still have to leave the current turn.
                // The one safe in-turn exception is a scoped debug/library
                // inspection of another Runtime-owned coroutine: the current
                // turn state remains borrowed and the target borrow cannot
                // escape this HRTB callback.
                if active_state != current_state
                    // SAFETY: `resolve` validated this unborrowed, stable arena
                    // pointer and no reference has yet been created for it.
                    || unsafe { state.as_ref() }.current_thread.is_none()
                {
                    return Err(StateResolveError::TurnBorrowActive {
                        slot: active_handle.slot(),
                        generation: active_handle.generation(),
                    });
                }
            }
            arena_ref.slots[handle.slot()].borrowed = true;
            let borrowed_slots = arena_ref.slots.iter().filter(|slot| slot.borrowed).count();
            arena_ref.peak_turn_borrowed_slots =
                arena_ref.peak_turn_borrowed_slots.max(borrowed_slots);
            state
        };
        let mut borrow = StateBorrow {
            arena,
            handle,
            state,
            kind: StateBorrowKind::Nested,
        };
        Ok(borrow.with_mut(f))
    }

    fn with_direct_state_mut<R>(
        mut arena: NonNull<Self>,
        handle: StateHandle,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        // SAFETY: Runtime passes its pinned arena while holding exclusive
        // access. The StateBorrow releases the slot even if `f` unwinds.
        let state = unsafe { arena.as_mut() }.begin_direct_borrow(handle)?;
        let mut borrow = StateBorrow {
            arena,
            handle,
            state,
            kind: StateBorrowKind::Nested,
        };
        Ok(borrow.with_mut(f))
    }

    /// Borrow exactly one state for one Runtime-owned execution turn.
    ///
    /// The returned value is owned and the state reference is confined to the
    /// HRTB callback. `StateBorrow` is dropped before this method returns,
    /// including while unwinding from a callback panic.
    fn with_turn_state_mut<R>(
        mut arena: NonNull<Self>,
        handle: StateHandle,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        let state = {
            // SAFETY: Runtime passes its pinned arena. No reference into the
            // arena is retained after this short acquisition scope.
            let arena_ref = unsafe { arena.as_mut() };
            arena_ref.begin_turn_borrow(handle)?
        };
        let mut borrow = StateBorrow {
            arena,
            handle,
            state,
            kind: StateBorrowKind::Turn,
        };
        Ok(borrow.with_mut(f))
    }

    fn reject_active_turn(&self) -> Result<(), StateResolveError> {
        if let Some(handle) = self.turn_borrow {
            return Err(StateResolveError::TurnBorrowActive {
                slot: handle.slot(),
                generation: handle.generation(),
            });
        }
        Ok(())
    }

    fn begin_turn_borrow(
        &mut self,
        handle: StateHandle,
    ) -> Result<NonNull<LuaState>, StateResolveError> {
        let (_, state) = self.validate(handle)?;
        self.reject_active_turn()?;

        if let Some((slot_index, slot)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.borrowed)
        {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }

        #[cfg(test)]
        self.turn_borrow_events.reserve(2);

        self.slots[handle.slot()].borrowed = true;
        self.turn_borrow = Some(handle);

        let borrowed_slots = self.slots.iter().filter(|slot| slot.borrowed).count();
        self.peak_turn_borrowed_slots = self.peak_turn_borrowed_slots.max(borrowed_slots);

        #[cfg(test)]
        {
            self.turn_borrow_events
                .push(TurnBorrowEvent::Acquired(handle));
        }

        Ok(state)
    }

    fn begin_direct_borrow(
        &mut self,
        handle: StateHandle,
    ) -> Result<NonNull<LuaState>, StateResolveError> {
        self.reject_active_turn()?;
        let (slot_index, state) = self.validate(handle)?;
        let slot = &mut self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        slot.borrowed = true;
        Ok(state)
    }

    fn release(&mut self, handle: StateHandle) -> bool {
        let Ok((slot_index, _)) = self.validate(handle) else {
            return false;
        };
        let was_borrowed = self.slots[slot_index].borrowed;
        self.slots[slot_index].borrowed = false;
        was_borrowed
    }

    fn release_turn(&mut self, handle: StateHandle) -> bool {
        if self.turn_borrow != Some(handle) {
            return false;
        }
        let released = self.release(handle);
        if released {
            self.turn_borrow = None;
            #[cfg(test)]
            {
                debug_assert!(
                    self.turn_borrow_events.len() < self.turn_borrow_events.capacity(),
                    "turn acquisition reserves its non-allocating release event"
                );
                self.turn_borrow_events
                    .push(TurnBorrowEvent::Released(handle));
            }
        }
        released
    }

    #[cfg(test)]
    fn remove_owned(&mut self, handle: StateHandle) -> Result<(), StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        let slot = &self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        if slot.is_main {
            return Err(StateResolveError::ArenaInvariant {
                reason: "the main state cannot be removed as a coroutine",
            });
        }

        let next_live_coroutine_states = self
            .live_coroutine_states
            .checked_sub(1)
            .expect("occupied coroutine slot requires a positive live count");
        self.vacate_slot(slot_index);
        self.live_coroutine_states = next_live_coroutine_states;
        // SAFETY: owned slots are created exclusively with Box::into_raw,
        // removed at most once, and have just been made vacant.
        unsafe {
            drop(Box::from_raw(state.as_ptr()));
        }
        Ok(())
    }

    /// Drain every arena-owned state in deterministic coroutine-before-main order.
    ///
    /// All occupied owned slots are preflighted for outstanding borrows before
    /// any mutation. Every state closes collector-validated open Upvalues
    /// before its slot generation is advanced or retired, then its Box is
    /// reconstructed and dropped while the collector heap is still alive.
    fn drain_owned(
        &mut self,
        gc: &mut GarbageCollector,
    ) -> Result<StateArenaDrainReport, StateResolveError> {
        self.validate_owned_drain()?;

        let mut report = StateArenaDrainReport::default();
        for drain_main in [false, true] {
            for slot_index in 0..self.slots.len() {
                let Some(state) = self.slots[slot_index].state else {
                    continue;
                };
                if self.slots[slot_index].is_main != drain_main {
                    continue;
                }

                #[cfg(test)]
                report
                    .drained_state_handles
                    .push(self.issue_handle(slot_index));

                if drain_main {
                    report.drained_main_states += 1;
                } else {
                    self.live_coroutine_states = self
                        .live_coroutine_states
                        .checked_sub(1)
                        .expect("occupied coroutine slot requires a positive live count");
                    report.drained_coroutine_states += 1;
                }
                // SAFETY: shutdown preflight proved that this occupied slot is
                // not borrowed. Its handle generation is still current while
                // the state closes every Upvalue that names it.
                let handle = self.issue_handle(slot_index);
                // SAFETY: shutdown preflight proved this occupied slot is unborrowed.
                let state_ref = unsafe { &mut *state.as_ptr() };
                let state_trace = state_ref.instruction_trace.clone();
                report
                    .state_shutdown
                    .merge(state_ref.prepare_for_runtime_shutdown(gc));
                if let Some(trace) = state_trace {
                    retire_trace_state(&trace, handle);
                }
                self.vacate_slot(slot_index);
                // SAFETY: each occupied slot originates from exactly one
                // Box::into_raw and was made vacant before reconstruction.
                drop(unsafe { Box::from_raw(state.as_ptr()) });
            }
        }

        debug_assert_eq!(self.live_coroutine_states, 0);
        Ok(report)
    }

    /// Close and remove arena-owned states that the canonical tracer did not
    /// reach during the current stop-the-world mark.
    ///
    /// The complete candidate set and free-list capacity are preflighted
    /// before the first state is mutated. Each state's open Upvalues close
    /// while its current StateHandle is still valid; only then is the slot
    /// generation advanced or permanently retired.
    fn sweep_unreachable_owned(
        &mut self,
        reachable: &HashSet<StateHandle>,
        gc: &mut GarbageCollector,
    ) -> Result<StateArenaCollectionReport, StateResolveError> {
        self.validate_owned_drain()?;

        let mut candidates = Vec::new();
        for slot_index in 0..self.slots.len() {
            let slot = &self.slots[slot_index];
            let Some(state) = slot.state else {
                continue;
            };
            if slot.is_main {
                continue;
            }
            let handle = self.issue_handle(slot_index);
            if !reachable.contains(&handle) {
                candidates.push((slot_index, handle, state));
            }
        }

        let reusable_slots = candidates
            .iter()
            .filter(|(slot_index, _, _)| self.slots[*slot_index].generation != u64::MAX)
            .count();
        self.free_slots.reserve(reusable_slots);
        let mut report = StateArenaCollectionReport {
            swept_state_handles: Vec::with_capacity(candidates.len()),
            state_shutdown: LuaStateShutdownReport::default(),
        };
        let expected_live_states = self
            .live_coroutine_states
            .checked_sub(candidates.len())
            .expect("unreachable candidates cannot exceed live coroutine states");

        for (slot_index, handle, state) in candidates {
            // SAFETY: preflight proved the slot is an unborrowed owned state.
            // Its current generation remains valid while open Upvalues close.
            // SAFETY: preflight proved this occupied slot is unborrowed.
            let state_ref = unsafe { &mut *state.as_ptr() };
            let state_trace = state_ref.instruction_trace.clone();
            report
                .state_shutdown
                .merge(state_ref.prepare_for_runtime_shutdown(gc));
            if let Some(trace) = state_trace {
                retire_trace_state(&trace, handle);
            }
            self.vacate_slot(slot_index);
            self.live_coroutine_states = self
                .live_coroutine_states
                .checked_sub(1)
                .expect("occupied coroutine slot requires a positive live count");
            // SAFETY: each owned slot originates from one Box::into_raw and
            // has just been made vacant before ownership is reconstructed.
            drop(unsafe { Box::from_raw(state.as_ptr()) });
            report.swept_state_handles.push(handle);
        }

        debug_assert_eq!(self.live_coroutine_states, expected_live_states);
        Ok(report)
    }

    fn validate_owned_drain(&self) -> Result<(), StateResolveError> {
        self.validate_internal_invariants()?;
        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot.state.is_some() && slot.borrowed {
                return Err(StateResolveError::AlreadyBorrowed {
                    slot: slot_index,
                    generation: slot.generation,
                });
            }
        }
        Ok(())
    }

    fn validate_internal_invariants(&self) -> Result<(), StateResolveError> {
        if self.runtime_resource_drain_cursor > self.runtime_resources.len() {
            return Err(StateResolveError::ArenaInvariant {
                reason: "runtime resource drain cursor exceeds the registry",
            });
        }
        let mut runtime_resource_ids = HashSet::new();
        for entry in &self.runtime_resources {
            if entry.object_id.is_null() || !runtime_resource_ids.insert(entry.object_id) {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "runtime resource identity is null or duplicated",
                });
            }
        }
        let mut temporary_root_ids = HashSet::new();
        for &slot_index in &self.free_slots {
            if slot_index >= self.slots.len() {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "free-list index is outside the slot table",
                });
            }
        }

        let mut occupied_coroutines = 0_usize;
        let mut occupied_main_states = 0_usize;
        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot.generation == 0 {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "slot generation is zero",
                });
            }

            let free_entries = self
                .free_slots
                .iter()
                .filter(|&&candidate| candidate == slot_index)
                .count();
            if free_entries > 1 {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "free-list contains a duplicate slot",
                });
            }

            if slot.retired {
                if slot.state.is_some()
                    || slot.is_main
                    || slot.borrowed
                    || slot.temporary_root_id.is_some()
                    || free_entries != 0
                    || slot.generation != u64::MAX
                {
                    return Err(StateResolveError::ArenaInvariant {
                        reason: "retired slot is occupied, reusable, or below generation max",
                    });
                }
                continue;
            }

            match slot.state {
                Some(_) => {
                    if free_entries != 0 {
                        return Err(StateResolveError::ArenaInvariant {
                            reason: "occupied slot appears in the free-list",
                        });
                    }
                    if slot.is_main {
                        occupied_main_states = occupied_main_states.checked_add(1).ok_or(
                            StateResolveError::ArenaInvariant {
                                reason: "occupied main-state count overflowed",
                            },
                        )?;
                    } else {
                        occupied_coroutines = occupied_coroutines.checked_add(1).ok_or(
                            StateResolveError::ArenaInvariant {
                                reason: "occupied coroutine-state count overflowed",
                            },
                        )?;
                    }
                    if let Some(temporary_root_id) = slot.temporary_root_id {
                        if slot.is_main {
                            return Err(StateResolveError::ArenaInvariant {
                                reason: "temporary state root names the main-state slot",
                            });
                        }
                        if temporary_root_id == 0 || !temporary_root_ids.insert(temporary_root_id) {
                            return Err(StateResolveError::ArenaInvariant {
                                reason: "temporary state-root identity is zero or duplicated",
                            });
                        }
                    }
                }
                None => {
                    if slot.is_main
                        || slot.borrowed
                        || slot.temporary_root_id.is_some()
                        || free_entries != 1
                    {
                        return Err(StateResolveError::ArenaInvariant {
                            reason: "vacant reusable slot has invalid flags or free-list count",
                        });
                    }
                }
            }
        }

        if occupied_coroutines != self.live_coroutine_states {
            return Err(StateResolveError::ArenaInvariant {
                reason: "live coroutine-state count disagrees with occupied slots",
            });
        }
        if occupied_main_states > 1 {
            return Err(StateResolveError::ArenaInvariant {
                reason: "state arena contains more than one main state",
            });
        }

        if let Some(handle) = self.turn_borrow {
            if handle.runtime_id() != self.runtime_id() {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "active turn handle belongs to another runtime",
                });
            }
            let Some(slot) = self.slots.get(handle.slot()) else {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "active turn handle points outside the slot table",
                });
            };
            if slot.retired
                || slot.generation != handle.generation()
                || slot.state.is_none()
                || !slot.borrowed
            {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "active turn handle disagrees with its borrowed slot",
                });
            }
            if self.slots.iter().filter(|slot| slot.borrowed).count() != 1 {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "active Runtime turn does not own exactly one state slot",
                });
            }
        }
        Ok(())
    }

    fn update_state_allocation(&mut self, handle: StateHandle, state: NonNull<LuaState>) -> bool {
        let Ok((slot_index, registered)) = self.validate(handle) else {
            return false;
        };
        if registered != state {
            return false;
        }
        // SAFETY: the scoped StateBorrow has ended its mutable callback, and
        // validation proved this exact occupied state allocation.
        self.slots[slot_index].accounted_bytes =
            unsafe { state.as_ref() }.allocator_payload_bytes();
        self.reconcile_allocation_account();
        true
    }

    fn reconcile_allocation_account(&mut self) {
        let state_bytes = self
            .slots
            .iter()
            .map(|slot| slot.accounted_bytes)
            .fold(0usize, usize::saturating_add);
        let resource_bytes = self
            .runtime_resources
            .capacity()
            .saturating_mul(std::mem::size_of::<RuntimeResourceEntry>());
        let live_bytes = state_bytes.saturating_add(resource_bytes);
        self.allocation_account.set_live_bytes(live_bytes);
    }

    fn vacate_slot(&mut self, slot_index: usize) {
        debug_assert!(
            self.turn_borrow
                .is_none_or(|handle| handle.slot() != slot_index),
            "an active Runtime turn cannot vacate its state"
        );
        let retires = self.slots[slot_index].generation == u64::MAX;
        if !retires {
            // Ensure the only potentially allocating operation happens before
            // the state pointer is cleared. If reserve panics, ownership and
            // generation remain unchanged.
            self.free_slots.reserve(1);
        }

        self.slots[slot_index].accounted_bytes = 0;
        self.reconcile_allocation_account();
        let slot = &mut self.slots[slot_index];
        debug_assert!(!slot.retired);
        slot.state = None;
        slot.is_main = false;
        slot.borrowed = false;
        slot.temporary_root_id = None;
        if retires {
            slot.retired = true;
        } else {
            slot.generation = slot
                .generation
                .checked_add(1)
                .expect("non-retiring generation can always advance");
            self.free_slots.push(slot_index);
        }
    }

    fn live_coroutine_state_count(&self) -> usize {
        self.live_coroutine_states
    }
}

impl Drop for StateArena {
    fn drop(&mut self) {
        self.turn_borrow = None;
        self.runtime_resources.clear();
        self.runtime_resource_drain_cursor = 0;
        for slot in &mut self.slots {
            let Some(state) = slot.state.take() else {
                continue;
            };
            // SAFETY: every occupied slot came from exactly one Box::into_raw
            // and StateArena is its unique final owner.
            unsafe {
                drop(Box::from_raw(state.as_ptr()));
            }
            slot.borrowed = false;
            slot.temporary_root_id = None;
        }
        self.live_coroutine_states = 0;
        self.allocation_account.set_live_bytes(0);
    }
}

/// Branded owner of one not-yet-published coroutine state.
///
/// The state is a canonical temporary root while this guard is live. Dropping
/// an uncommitted guard removes the arena slot, advances or retires its
/// generation exactly once, and drops the unpublished state allocation.
pub struct PendingState<'scope> {
    arena: NonNull<StateArena>,
    publisher: NonNull<LuaState>,
    handle: StateHandle,
    temporary_root_id: u64,
    bound_thread: Option<ObjectId>,
    committed: bool,
    _scope: PhantomData<&'scope mut ()>,
}

impl PendingState<'_> {
    /// Install and validate the bidirectional State↔Thread edge while the
    /// Thread remains protected by its object-publication transaction.
    pub fn bind_thread<'publication>(
        &mut self,
        transaction: &mut PublicationTxn<'publication>,
        thread: Rooted<'publication, Thread>,
    ) -> Result<(), PendingStateError> {
        let thread_id = thread.object_id();
        transaction.with_mut(&thread, |thread| {
            thread.set_state_handle(self.handle);
        })?;
        let mut arena = self.arena;
        // SAFETY: the callback installs the Thread in a canonical pending
        // state root before the object temporary root is released.
        unsafe {
            transaction.publish_thread_value(thread, |value| {
                let Value::Thread(thread_ref) = value else {
                    unreachable!("typed Thread publication produced another Value kind");
                };
                // SAFETY: PendingState is created only from this pinned arena
                // and remains the unique owner of the unpublished slot.
                arena
                    .as_mut()
                    .bind_pending_thread(self.handle, self.temporary_root_id, thread_ref)
            })
        }??;
        self.bound_thread = Some(thread_id);
        Ok(())
    }

    /// Publish the bound Thread directly into a Lua stack and atomically
    /// release the temporary state root.
    pub fn publish_thread_to_stack(
        &mut self,
        publisher: &mut LuaState,
    ) -> Result<StateHandle, PendingStateError> {
        self.validate_publisher(publisher)?;
        let thread_id = self.bound_thread.ok_or(PendingStateError::ThreadNotBound)?;
        let arena = self.arena;
        // SAFETY: the branded guard's pinned arena is live for this callback.
        let thread = unsafe { arena.as_ref() }.pending_thread(
            self.handle,
            self.temporary_root_id,
            thread_id,
        )?;
        let original_top = publisher.top;
        publisher.push_value(Value::Thread(thread));
        let commit = self.commit();
        if commit.is_err() {
            publisher.stack.set_top(original_top);
            publisher.top = original_top;
        }
        commit
    }

    /// Publish a protected wrapper Function that retains the bound Thread
    /// through a closed Upvalue, then atomically release the temporary state
    /// root.
    pub fn publish_wrapper_to_stack<'publication>(
        &mut self,
        transaction: &mut PublicationTxn<'publication>,
        wrapper: Rooted<'publication, Function>,
        publisher: &mut LuaState,
    ) -> Result<StateHandle, PendingStateError> {
        self.validate_publisher(publisher)?;
        let thread_id = self.bound_thread.ok_or(PendingStateError::ThreadNotBound)?;
        let arena = self.arena;
        // SAFETY: the branded guard's pinned arena is live for this callback.
        unsafe { arena.as_ref() }.pending_thread(self.handle, self.temporary_root_id, thread_id)?;
        if !transaction.function_reaches_thread_object_id(&wrapper, thread_id)? {
            return Err(PendingStateError::WrapperThreadMismatch);
        }
        let original_top = publisher.top;
        // SAFETY: the exact publisher stack is a canonical Runtime root. The
        // callback writes the Function before committing the state root.
        let commit = unsafe {
            transaction.publish_function_value(wrapper, |value| {
                publisher.push_value(value);
                self.commit()
            })
        }?;
        if commit.is_err() {
            publisher.stack.set_top(original_top);
            publisher.top = original_top;
        }
        commit
    }

    fn validate_publisher(&self, publisher: &mut LuaState) -> Result<(), PendingStateError> {
        if NonNull::from(publisher) != self.publisher {
            return Err(PendingStateError::PublisherMismatch);
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<StateHandle, PendingStateError> {
        let mut arena = self.arena;
        // SAFETY: the lexical guard is the only capability that can commit
        // this exact temporary-root identity.
        unsafe { arena.as_mut() }.commit_pending(self.handle, self.temporary_root_id)?;
        self.committed = true;
        Ok(self.handle)
    }
}

impl Drop for PendingState<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut arena = self.arena;
        // SAFETY: PendingState cannot outlive the Runtime-owned pinned arena;
        // an uncommitted guard still uniquely owns this unpublished slot.
        if unsafe { arena.as_mut() }
            .rollback_pending(self.handle, self.temporary_root_id)
            .is_err()
        {
            // Fail closed: never guess another slot/root identity during
            // unwind. Shutdown will still drain any retained owned state.
            // SAFETY: same pinned-arena lifetime as above.
            let arena = unsafe { arena.as_mut() };
            arena.rejected_temporary_root_releases =
                arena.rejected_temporary_root_releases.saturating_add(1);
        }
    }
}

struct StateBorrow {
    arena: NonNull<StateArena>,
    handle: StateHandle,
    state: NonNull<LuaState>,
    kind: StateBorrowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateBorrowKind {
    Nested,
    Turn,
}

impl StateBorrow {
    fn with_mut<R>(&mut self, f: impl for<'state> FnOnce(&'state mut LuaState) -> R) -> R {
        // SAFETY: StateArena marked this generation as exclusively borrowed;
        // the HRTB closure prevents the reference from escaping this call.
        unsafe { f(self.state.as_mut()) }
    }
}

impl Drop for StateBorrow {
    fn drop(&mut self) {
        let mut arena = self.arena;
        // SAFETY: StateBorrow never escapes the resolving LuaState call. The
        // pinned RuntimeStorage and its StateArena therefore still exist.
        let updated =
            // SAFETY: the HRTB callback has returned, so no state reference
            // remains while the arena reads this exact allocation's sizes.
            unsafe { arena.as_mut() }.update_state_allocation(self.handle, self.state);
        debug_assert!(updated);
        let released = match self.kind {
            // SAFETY: StateBorrow never escapes the resolving LuaState call.
            StateBorrowKind::Nested => unsafe { arena.as_mut() }.release(self.handle),
            // SAFETY: the pinned arena outlives this scoped turn borrow.
            StateBorrowKind::Turn => unsafe { arena.as_mut() }.release_turn(self.handle),
        };
        debug_assert!(released);
    }
}

impl LuaState {
    pub(crate) fn attach_runtime_state(
        &mut self,
        handle: StateHandle,
        arena: Option<NonNull<UnsafeCell<StateArena>>>,
    ) {
        self.state_handle = Some(handle);
        self.state_arena = arena;
    }

    /// Return this state's runtime-scoped generational handle.
    pub fn state_handle(&self) -> Option<StateHandle> {
        self.state_handle
    }

    /// Reserve non-allocating publication capacity for Runtime resource hooks.
    ///
    /// Standalone states return `Ok(false)` because they have no Runtime
    /// shutdown service. Runtime-attached callers must reserve before making
    /// the corresponding object graph Lua-visible, so allocator failure
    /// cannot leave a published but unregistered external resource.
    pub fn reserve_runtime_resource_registrations(
        &mut self,
        additional: usize,
    ) -> Result<bool, StateResolveError> {
        let Some(arena_cell) = self.state_arena else {
            return Ok(false);
        };
        // SAFETY: Runtime attaches only its pinned StateArena UnsafeCell. The
        // state-side arena protocol permits this short service mutation while
        // the current state slot is borrowed; no arena reference escapes.
        let arena = unsafe { &mut *arena_cell.as_ref().get() };
        arena.reserve_runtime_resources(additional)?;
        Ok(true)
    }

    /// Register one weak collector identity for deterministic Runtime close.
    ///
    /// The registration adds no GC root. Capacity must have been reserved
    /// before the resource became Lua-visible. Duplicate identities are
    /// ignored. Standalone states return `Ok(false)`.
    pub fn register_runtime_resource(
        &mut self,
        object_id: ObjectId,
        drain: RuntimeResourceDrainHook,
    ) -> Result<bool, StateResolveError> {
        let Some(arena_cell) = self.state_arena else {
            return Ok(false);
        };
        // SAFETY: same pinned arena and state-side service protocol as the
        // reservation method above.
        unsafe { &mut *arena_cell.as_ref().get() }.register_runtime_resource(object_id, drain)
    }

    /// Move a coroutine state into this state's Runtime-owned arena.
    ///
    /// Standalone `LuaState` values are intentionally rejected: coroutine
    /// states require a Runtime owner so their allocation cannot leak.
    #[cfg(test)]
    pub(crate) fn insert_coroutine_state(
        &mut self,
        state: LuaState,
    ) -> Result<StateHandle, StateResolveError> {
        let arena_cell = self
            .state_arena
            .ok_or(StateResolveError::ArenaUnavailable)?;
        // SAFETY: Runtime attaches only its pinned StateArena UnsafeCell.
        // The current LuaState is alive for this call, so the Runtime and arena
        // cannot be dropped while the child Box is transferred.
        let mut arena = NonNull::new(unsafe { arena_cell.as_ref() }.get())
            .expect("UnsafeCell::get never returns null");
        // SAFETY: `arena` was derived from this state's attached `UnsafeCell`,
        // and the runtime arena protocol gives this call exclusive access.
        let handle = unsafe { arena.as_mut() }.insert_owned_with_temporary_root(
            Box::new(state),
            None,
            Some(arena_cell),
            false,
        )?;
        Ok(handle)
    }

    /// Execute one branded coroutine-state publication transaction.
    ///
    /// The inserted child is independently traced through
    /// `TEMPORARY_STATE_ROOTS` until the callback publishes a checked
    /// State↔Thread graph and commits it. Returning or unwinding without a
    /// commit rolls the arena slot back.
    pub fn with_pending_coroutine_state<R>(
        &mut self,
        state: LuaState,
        publish: impl for<'scope> FnOnce(&mut PendingState<'scope>, &mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        let arena_cell = self
            .state_arena
            .ok_or(StateResolveError::ArenaUnavailable)?;
        // SAFETY: Runtime installs only its pinned StateArena UnsafeCell. The
        // branded guard is dropped before this method returns or unwinds, and
        // Runtime's dynamic arena protocol excludes another mutable borrow.
        let mut arena = NonNull::new(unsafe { arena_cell.as_ref() }.get())
            .expect("UnsafeCell::get never returns null");
        // SAFETY: `arena` was derived from this state's attached `UnsafeCell`,
        // and no other mutable arena access can overlap the branded guard.
        let (handle, temporary_root_id) =
            unsafe { arena.as_mut() }.insert_pending_owned(Box::new(state), Some(arena_cell))?;
        let mut pending = PendingState {
            arena,
            publisher: NonNull::from(&mut *self),
            handle,
            temporary_root_id,
            bound_thread: None,
            committed: false,
            _scope: PhantomData,
        };
        Ok(publish(&mut pending, self))
    }

    /// Number of lexical temporary state roots in this Runtime arena.
    pub fn temporary_state_root_count(&self) -> usize {
        let Some(arena_cell) = self.state_arena else {
            return 0;
        };
        // SAFETY: StateArena is pinned and this shared diagnostic does not
        // create a LuaState reference or mutate arena state.
        unsafe { &*arena_cell.as_ref().get() }.temporary_state_root_count()
    }

    /// Number of temporary state-root release mismatches observed by this
    /// Runtime arena.
    pub fn rejected_temporary_state_root_release_count(&self) -> usize {
        let Some(arena_cell) = self.state_arena else {
            return 0;
        };
        // SAFETY: same pinned shared diagnostic invariant as above.
        unsafe { &*arena_cell.as_ref().get() }.rejected_temporary_root_releases
    }

    /// Resolve another state and confine its mutable reference to `f`.
    ///
    /// The target must belong to the same Runtime and must differ from this
    /// currently borrowed state. Nested borrows are tracked by slot, so a caller
    /// or other active coroutine cannot be aliased. During a Runtime execution
    /// turn this supports scoped debug/library inspection of another owned
    /// coroutine; it is not an execution-switch API.
    pub fn with_resolved_state_mut<R>(
        &mut self,
        handle: StateHandle,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        let active_services = self
            .active_gc_ptr()
            .and_then(NonNull::new)
            .zip(self.active_string_pool_ptr().and_then(NonNull::new));
        let arena_cell = self
            .state_arena
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let current_state = NonNull::from(&mut *self);
        // SAFETY: state_arena is installed only from the pinned RuntimeStorage
        // UnsafeCell. Runtime's arena borrow protocol excludes overlap.
        // StateArena validates target identity and guards the mutable borrow
        // until the higher-ranked closure returns.
        let arena = NonNull::new(unsafe { arena_cell.as_ref() }.get())
            .expect("UnsafeCell::get never returns null");
        StateArena::with_state_mut(arena, handle, current_state, |target| {
            // Native debug APIs inspect another coroutine while the caller's
            // Runtime turn remains active. Switch the dynamic service identity
            // for exactly this resolved-state borrow so validated Proto/String
            // reads see the same Heap without installing persistent pointers on
            // either LuaState.
            let _target_context = active_services.map(|(gc, strings)| {
                // SAFETY: the Runtime arena confines `target` to this HRTB
                // callback, and the service pointers are borrowed from the
                // caller's still-live dynamic Runtime context.
                unsafe { enter_vm_context(NonNull::from(&mut *target), gc, strings) }
            });
            f(target)
        })
    }
}

/// Stable allocation for the Heap, state arena, and Runtime-scoped services.
struct RuntimeStorage {
    state_arena: UnsafeCell<StateArena>,
    heap: Heap,
    native_activations: NativeActivationStack,
    incremental_trace: Option<IncrementalRootTrace>,
    _pin: PhantomPinned,
}

const FIXED_RUNTIME_STRING_BYTES: [&[u8]; 3] =
    [b"not enough memory", b"error in error handling", b"_ENV"];

impl RuntimeStorage {
    fn new(handle_issuer: StateHandleIssuer) -> Self {
        let heap = Heap::new();
        let allocator = heap.allocation_ledger();
        Self {
            state_arena: UnsafeCell::new(StateArena::new(handle_issuer, allocator)),
            heap,
            native_activations: NativeActivationStack::default(),
            incremental_trace: None,
            _pin: PhantomPinned,
        }
    }

    fn state_arena(&self) -> &StateArena {
        // SAFETY: shared diagnostics never overlap a Runtime-owned mutable
        // arena operation.
        unsafe { &*self.state_arena.get() }
    }

    fn state_arena_mut(&mut self) -> &mut StateArena {
        self.state_arena.get_mut()
    }

    fn state_arena_cell(&self) -> NonNull<UnsafeCell<StateArena>> {
        NonNull::from(&self.state_arena)
    }
}

/// Unique owner of one main Lua state and its runtime services.
///
/// The `Rc` marker intentionally makes this type neither `Send` nor `Sync`.
/// Runtime APIs also compare the current thread with `owner_thread` so that a
/// future accidental relaxation of auto-traits fails at the API boundary.
pub struct Runtime {
    id: RuntimeId,
    owner_thread: ThreadId,
    phase: RuntimePhase,
    active_executions: usize,
    main_state_handle: Option<StateHandle>,
    global_root: Option<GcRef<Table>>,
    registry_root: Option<GcRef<Table>>,
    fixed_strings: Vec<GcRef<GcString>>,
    shutdown_summary: Option<RuntimeShutdownSummary>,
    runtime_fault_injection: Option<RuntimeFaultInjection>,
    instruction_trace: Option<SharedInstructionTrace>,
    heap: Pin<Box<RuntimeStorage>>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Runtime {
    /// Create a running runtime with a rooted global table and main state.
    ///
    /// Each LuaState receives scoped Heap service access only while Runtime
    /// holds its exclusive turn borrow.
    ///
    /// # Panics
    ///
    /// Panics if the process-wide RuntimeId namespace is exhausted. Hosts that
    /// need to report that terminal condition should call [`Runtime::try_new`].
    #[allow(
        clippy::new_without_default,
        reason = "Default would hide terminal RuntimeId exhaustion; hosts can use try_new"
    )]
    pub fn new() -> Self {
        Self::try_new().expect("process-wide RuntimeId namespace is exhausted")
    }

    /// Try to create a running runtime without allowing RuntimeId reuse.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeIdExhausted`] after the process-wide monotonic identity
    /// namespace is exhausted. No heap, roots, or state slots are allocated on
    /// that failure path.
    pub fn try_new() -> Result<Self, RuntimeIdExhausted> {
        let handle_issuer = StateHandleIssuer::try_new()?;
        let id = handle_issuer.runtime_id();
        let mut heap = Box::pin(RuntimeStorage::new(handle_issuer));

        // SAFETY: `RuntimeStorage` is pinned for the remainder of the Runtime's
        // life. We take field addresses and never move out of the heap.
        let heap_mut = unsafe { Pin::get_unchecked_mut(heap.as_mut()) };
        let (global_root, registry_root, fixed_strings) =
            heap_mut.heap.with_parts_mut(|gc, strings| {
                let fixed_strings = METAMETHOD_NAMES
                    .iter()
                    .map(|name| name.as_bytes())
                    .chain(FIXED_RUNTIME_STRING_BYTES)
                    .map(|bytes| {
                        let string = strings.intern_bytes(gc, bytes);
                        gc.with_ref(string, GcString::mark_fixed)
                            .expect("new fixed Runtime string remains registered");
                        string
                    })
                    .collect();
                let (global_root, registry_root) = gc.with_publication(|transaction| {
                    let global = transaction.alloc(Table::new());
                    let registry = transaction.alloc(Table::new());
                    let global = transaction
                        .publish_as_explicit_root(global)
                        .expect("new Runtime global Table remains registered");
                    let registry = transaction
                        .publish_as_explicit_root(registry)
                        .expect("new Runtime registry Table remains registered");
                    (global, registry)
                });
                (global_root, registry_root, fixed_strings)
            });

        let mut main_state = Box::new(LuaState::with_global_table(global_root));
        main_state.registry = Some(registry_root);
        let arena_cell = heap_mut.state_arena_cell();
        let main_state_handle = heap_mut
            .state_arena_mut()
            .insert_main_owned(main_state, arena_cell)
            .expect("fresh Runtime main-state insertion cannot be fault-injected");

        Ok(Self {
            id,
            owner_thread: thread::current().id(),
            phase: RuntimePhase::Running,
            active_executions: 0,
            main_state_handle: Some(main_state_handle),
            global_root: Some(global_root),
            registry_root: Some(registry_root),
            fixed_strings,
            shutdown_summary: None,
            runtime_fault_injection: None,
            instruction_trace: None,
            heap,
            _not_send_or_sync: PhantomData,
        })
    }

    /// Return this runtime's process-unique diagnostic identifier.
    pub fn id(&self) -> RuntimeId {
        self.id
    }

    /// Return the thread that created and owns this runtime.
    pub fn owner_thread_id(&self) -> ThreadId {
        self.owner_thread
    }

    /// Return the current lifecycle phase.
    pub fn phase(&self) -> RuntimePhase {
        self.phase
    }

    /// Return the number of outstanding execution-part guards.
    pub fn active_execution_count(&self) -> usize {
        self.active_executions
    }

    /// Return the main global-table root while the runtime is open.
    pub fn global_root(&self) -> Option<GcRef<Table>> {
        self.global_root
    }

    /// Return the Runtime-owned persistent registry root while open.
    pub fn registry_root(&self) -> Option<GcRef<Table>> {
        self.registry_root
    }

    /// Read one live Runtime-owned Lua string within a checked Heap scope.
    pub fn with_string_bytes<R>(
        &self,
        string: GcRef<GcString>,
        read: impl for<'bytes> FnOnce(&'bytes [u8]) -> R,
    ) -> Result<R, GcRefValidationError> {
        if self.check_owner().is_err() || self.phase == RuntimePhase::Closed {
            return Err(GcRefValidationError::CollectorUnavailable);
        }
        self.heap
            .as_ref()
            .get_ref()
            .heap
            .collector()
            .with_string_bytes(string, read)
    }

    /// Copy one live Runtime-owned Lua string.
    pub fn copy_string_bytes(
        &self,
        string: GcRef<GcString>,
    ) -> Result<Vec<u8>, GcRefValidationError> {
        self.with_string_bytes(string, <[u8]>::to_vec)
    }

    /// Resolve and validate one canonical interned string without allocating.
    pub fn find_interned_string(
        &self,
        bytes: &[u8],
    ) -> Result<Option<GcRef<GcString>>, GcRefValidationError> {
        if self.check_owner().is_err() || self.phase == RuntimePhase::Closed {
            return Err(GcRefValidationError::CollectorUnavailable);
        }
        let heap = &self.heap.as_ref().get_ref().heap;
        let Some(string) = heap.strings().find_bytes(bytes) else {
            return Ok(None);
        };
        heap.collector().with_ref(string, |_| ())?;
        Ok(Some(string))
    }

    /// Return the arena-owned main state while the runtime remains open.
    pub fn main_state(&self) -> Option<&LuaState> {
        let handle = self.main_state_handle?;
        self.heap
            .as_ref()
            .get_ref()
            .state_arena()
            .main_state_ref(handle)
            .ok()
    }

    fn with_main_state_mut<R>(
        &mut self,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, RuntimeAccessError> {
        let handle = self
            .main_state_handle
            .ok_or(RuntimeAccessError::MainStateUnavailable {
                runtime_id: self.id,
            })?;
        let runtime_id = self.id;
        // SAFETY: RuntimeStorage remains pinned and this method owns the
        // exclusive Runtime borrow for the complete scoped state callback.
        let storage = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let arena = NonNull::new(storage.state_arena.get())
            .expect("pinned StateArena UnsafeCell never returns null");
        StateArena::with_direct_state_mut(arena, handle, f)
            .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })
    }

    /// Return coroutine states currently retained by the StateArena.
    pub fn live_coroutine_state_count(&self) -> usize {
        self.heap
            .as_ref()
            .get_ref()
            .state_arena()
            .live_coroutine_state_count()
    }

    /// Snapshot managed allocator live/peak/cumulative payload bytes.
    pub fn allocator_stats(&self) -> AllocatorStats {
        self.heap.as_ref().get_ref().heap.allocator_stats()
    }

    /// Return scheduler depth and cleanup invariants for the last execution.
    pub fn activation_stats(&self) -> RuntimeActivationStats {
        let heap = self.heap.as_ref().get_ref();
        heap.native_activations
            .snapshot(heap.state_arena().peak_turn_borrowed_slots)
    }

    /// Begin a runtime-scoped pre-instruction register-snapshot stream.
    ///
    /// The writer is shared by every state turn in this Runtime, so coroutine
    /// switches retain one monotonically increasing event sequence.
    pub fn start_instruction_trace(
        &mut self,
        output: Box<dyn Write>,
    ) -> Result<(), RuntimeAccessError> {
        self.start_instruction_trace_with_mode(output, InstructionTraceMode::Snapshot)
    }

    /// Begin a runtime-scoped instruction trace with explicit snapshot or diff evidence.
    pub fn start_instruction_trace_with_mode(
        &mut self,
        output: Box<dyn Write>,
        mode: InstructionTraceMode,
    ) -> Result<(), RuntimeAccessError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            });
        }
        if self.active_executions != 0 {
            return Err(RuntimeAccessError::ActiveExecutions {
                runtime_id: self.id,
                count: self.active_executions,
            });
        }
        self.instruction_trace = Some(Rc::new(RefCell::new(InstructionTrace::new(output, mode))));
        Ok(())
    }

    /// Disable and flush the active instruction trace stream.
    pub fn finish_instruction_trace(
        &mut self,
    ) -> Result<Option<InstructionTraceSummary>, InstructionTraceError> {
        let Some(trace) = self.instruction_trace.take() else {
            return Ok(None);
        };
        trace.borrow_mut().finish().map(Some)
    }

    /// Install a one-shot Runtime scheduler failure for durability tests.
    #[doc(hidden)]
    pub fn inject_runtime_fault_after(
        &mut self,
        site: RuntimeFaultSite,
        successful_matching_checkpoints: usize,
    ) {
        self.runtime_fault_injection = Some(RuntimeFaultInjection {
            site,
            successful_matching_checkpoints,
        });
    }

    /// Remove a pending Runtime scheduler durability-test failure.
    #[doc(hidden)]
    pub fn clear_runtime_fault_injection(&mut self) {
        self.runtime_fault_injection = None;
    }

    /// Install a one-shot managed-allocation failure plan for durability tests.
    ///
    /// The selected checkpoint fails before changing its object publication or
    /// StateArena owner graph. This is intentionally separate from the future
    /// public Lua allocator callback.
    #[doc(hidden)]
    pub fn inject_allocator_failure_after(
        &self,
        site: AllocationSite,
        successful_matching_checkpoints: usize,
    ) {
        self.heap
            .as_ref()
            .get_ref()
            .heap
            .inject_allocator_failure_after(site, successful_matching_checkpoints);
    }

    /// Remove a durability-test allocation failure plan.
    #[doc(hidden)]
    pub fn clear_allocator_failure_injection(&self) {
        self.heap
            .as_ref()
            .get_ref()
            .heap
            .clear_allocator_failure_injection();
    }

    /// Return active lexical temporary state roots.
    pub fn temporary_state_root_count(&self) -> usize {
        self.heap
            .as_ref()
            .get_ref()
            .state_arena()
            .temporary_state_root_count()
    }

    /// Return rejected exact-id temporary state-root releases.
    pub fn rejected_temporary_state_root_release_count(&self) -> usize {
        self.heap
            .as_ref()
            .get_ref()
            .state_arena()
            .rejected_temporary_root_releases
    }

    /// Verify that the caller is executing on the owning thread.
    pub fn check_owner(&self) -> Result<(), RuntimeAccessError> {
        let current = thread::current().id();
        if current == self.owner_thread {
            Ok(())
        } else {
            Err(RuntimeAccessError::WrongThread {
                runtime_id: self.id,
                owner: self.owner_thread,
                current,
            })
        }
    }

    /// Borrow the main state, collector, and string pool for one execution.
    ///
    /// The returned guard increments the active-execution count and decrements
    /// it on drop. It cannot outlive or be used to close the runtime.
    pub fn parts_mut(&mut self) -> Result<RuntimePartsMut<'_>, RuntimeAccessError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            });
        }

        let state_handle =
            self.main_state_handle
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;

        // SAFETY: the heap was pinned in `new` and no API moves its fields out.
        // Creating disjoint mutable borrows of its two service fields is valid
        // for the lifetime of the exclusive Runtime borrow.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let next_active_executions = self
            .active_executions
            .checked_add(1)
            .expect("runtime active-execution counter overflow");
        let mut arena = NonNull::new(heap.state_arena.get())
            .expect("pinned StateArena UnsafeCell never returns null");
        // SAFETY: parts_mut holds the exclusive Runtime borrow, and this guard
        // releases the arena slot before that borrow ends.
        let mut state_ptr = unsafe { arena.as_mut() }
            .begin_direct_borrow(state_handle)
            .map_err(|source| RuntimeAccessError::StateArena {
                runtime_id: self.id,
                source,
            })?;
        self.active_executions = next_active_executions;

        let (gc, string_pool) = heap.heap.parts_mut();
        // SAFETY: the arena owns this allocation and marked the exact handle
        // exclusively borrowed until RuntimePartsMut releases it on Drop.
        let state = unsafe { state_ptr.as_mut() };
        Ok(RuntimePartsMut {
            state,
            gc,
            string_pool,
            state_arena: arena,
            state_handle,
            active_executions: &mut self.active_executions,
        })
    }

    /// Drive state execution as a sequence of non-overlapping turns.
    ///
    /// This M1 substrate deliberately does not implement coroutine semantics.
    /// It establishes the Runtime ownership rule the coroutine trampoline will
    /// use: the callback receives exactly one validated `LuaState`, returns an
    /// owned [`RuntimeTurn`], and the state guard is dropped before a requested
    /// next handle is resolved. The session counts as one active execution even
    /// when it spans many turns.
    ///
    /// A callback panic releases both the state slot and active-execution count
    /// during unwinding. Foreign, stale, retired, vacant, or already-borrowed
    /// handles fail closed through [`RuntimeAccessError::StateArena`].
    #[allow(
        dead_code,
        reason = "M1 Runtime turn substrate is integrated before coroutine dispatch"
    )]
    pub(crate) fn drive_state_turns<T>(
        &mut self,
        initial: StateHandle,
        mut execute_turn: impl for<'turn> FnMut(StateHandle, &'turn mut LuaState) -> RuntimeTurn<T>,
    ) -> Result<T, RuntimeAccessError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            });
        }
        if self.main_state_handle.is_none() {
            return Err(RuntimeAccessError::MainStateUnavailable {
                runtime_id: self.id,
            });
        }
        if self.active_executions != 0 {
            return Err(RuntimeAccessError::ActiveExecutions {
                runtime_id: self.id,
                count: self.active_executions,
            });
        }

        let next_active_executions = self
            .active_executions
            .checked_add(1)
            .expect("runtime active-execution counter overflow");
        self.active_executions = next_active_executions;
        let _active_execution = ActiveExecutionGuard {
            active_executions: &mut self.active_executions,
        };
        let runtime_id = self.id;
        let instruction_trace = self.instruction_trace.clone();

        // SAFETY: RuntimeStorage was pinned during construction and no API moves
        // its fields. The arena and service fields are disjoint, and state
        // references are confined to one HRTB callback at a time.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let arena = NonNull::new(heap.state_arena.get())
            .expect("pinned StateArena UnsafeCell never returns null");
        let (gc, string_pool) = heap.heap.parts_mut();
        let mut current = initial;

        loop {
            let outcome = StateArena::with_turn_state_mut(arena, current, |state| {
                state.instruction_trace = instruction_trace.clone();
                with_vm_context(state, gc, string_pool, |state| execute_turn(current, state))
            })
            .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })?;

            match outcome {
                RuntimeTurn::Switch(next) => current = next,
                RuntimeTurn::Complete(value) => return Ok(value),
            }
        }
    }

    /// Execute a Proto through the Runtime-owned coroutine trampoline.
    ///
    /// Sealed Runtime-native operations publish owned requests and return
    /// [`VmExit::NativeRequest`]. The current state borrow ends before this
    /// driver validates and borrows the requested target. Deferred call frames
    /// and transfer values remain in the Runtime-owned activation stack until
    /// the target yields, returns, or errors.
    pub fn execute_proto(
        &mut self,
        proto: GcRef<Proto>,
    ) -> Result<ExecResult, RuntimeExecutionError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            }
            .into());
        }
        let initial = self
            .main_state_handle
            .ok_or(RuntimeAccessError::MainStateUnavailable {
                runtime_id: self.id,
            })?;
        if self.active_executions != 0 {
            return Err(RuntimeAccessError::ActiveExecutions {
                runtime_id: self.id,
                count: self.active_executions,
            }
            .into());
        }

        self.active_executions = self
            .active_executions
            .checked_add(1)
            .expect("runtime active-execution counter overflow");
        let _active_execution = ActiveExecutionGuard {
            active_executions: &mut self.active_executions,
        };
        let runtime_id = self.id;
        let runtime_fault_injection = &mut self.runtime_fault_injection;
        let instruction_trace = self.instruction_trace.clone();

        // SAFETY: RuntimeStorage was pinned at construction. The arena, collector,
        // and activation stack are disjoint fields and remain at stable
        // addresses for the whole execution session.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        if !heap.native_activations.frames.is_empty()
            || !heap.native_activations.upvalue_transfers.is_empty()
            || !heap.native_activations.debug_upvalue_transfers.is_empty()
            || !heap.native_activations.protected_calls.is_empty()
            || !heap.native_activations.callback_helpers.is_empty()
            || !heap.native_activations.gc_frames.is_empty()
        {
            return Err(RuntimeExecutionError::Protocol(
                "activation stack was not empty at execution entry".to_string(),
            ));
        }
        let mut arena = NonNull::new(heap.state_arena.get())
            .expect("pinned StateArena UnsafeCell never returns null");
        let (gc, string_pool) = heap.heap.parts_mut();
        let activations = &mut heap.native_activations;
        let incremental_trace = &mut heap.incremental_trace;
        // SAFETY: execute_proto owns the Runtime, no state turn has started,
        // and the pinned arena is disjoint from the activation stack.
        unsafe { arena.as_mut() }.peak_turn_borrowed_slots = 0;
        activations.begin_session();
        gc.begin_automatic_checkpoint_driver();
        let activation_cleanup = NativeActivationSessionGuard {
            activations,
            gc,
            arena,
        };
        let activations = &mut *activation_cleanup.activations;
        let gc = &mut *activation_cleanup.gc;

        let mut current = initial;
        let mut action = NativeDriverAction::StartProto(proto);
        let mut pending_native_delivery = None;
        loop {
            activations.observe_turn();
            activations.seed_roots(gc);
            let driver_role = action.role();
            let completed_activation = match &action {
                NativeDriverAction::Deliver {
                    frame,
                    replay_dead_ancestor,
                    ..
                } => Some((
                    frame.request.id,
                    replay_dead_ancestor
                        .then_some(frame.caller_thread)
                        .flatten(),
                )),
                _ => pending_native_delivery.take(),
            };
            let completed_upvalue_transfer =
                matches!(&action, NativeDriverAction::DeliverUpvalue { .. });
            let completed_debug_upvalue_transfer =
                matches!(&action, NativeDriverAction::DeliverDebugUpvalue { .. });
            let completed_protected_call =
                matches!(&action, NativeDriverAction::FinishProtectedCall { .. });
            let completed_callback_helper =
                matches!(&action, NativeDriverAction::FinishCallbackHelper { .. });
            let completed_collection =
                matches!(&action, NativeDriverAction::DeliverCollection { .. });
            let mut event = StateArena::with_turn_state_mut(arena, current, |state| {
                state.instruction_trace = instruction_trace.clone();
                with_vm_context_gc(state, gc, string_pool, |state, gc| {
                    state
                        .with_native_request_scope(|state| {
                            let vm_result = execute_native_driver_action(state, gc, action);
                            classify_native_turn(state, gc, vm_result, driver_role)
                        })
                        .map_err(|error| {
                            RuntimeError::new(format!(
                                "Runtime-native mailbox scope failed: {error:?}"
                            ))
                        })
                        .and_then(std::convert::identity)
                })
            })
            .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })??;
            if matches!(event, NativeTurnEvent::Request { .. })
                && RuntimeFaultInjection::checkpoint(
                    runtime_fault_injection,
                    RuntimeFaultSite::SealedMailboxTake,
                )
            {
                return Err(RuntimeExecutionError::Protocol(
                    "injected Runtime fault at SealedMailboxTake".to_string(),
                ));
            }
            if completed_upvalue_transfer {
                activations.upvalue_transfers.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "Upvalue delivery lost its rooted transfer frame".to_string(),
                    )
                })?;
            }
            if completed_debug_upvalue_transfer {
                activations.debug_upvalue_transfers.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "debug Upvalue delivery lost its rooted transfer frame".to_string(),
                    )
                })?;
            }
            if completed_protected_call {
                activations.protected_calls.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "protected helper delivery lost its rooted activation frame".to_string(),
                    )
                })?;
            }
            if completed_callback_helper {
                activations.callback_helpers.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "callback helper delivery lost its rooted activation frame".to_string(),
                    )
                })?;
            }
            if completed_collection {
                activations.gc_frames.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "full collection delivery lost its rooted activation frame".to_string(),
                    )
                })?;
            }

            if matches!(event, NativeTurnEvent::UpvalueAccess(_)) {
                pending_native_delivery = completed_activation;
            } else if let Some((request_id, replay_dead_thread)) = completed_activation {
                let completed = activations.frames.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "deferred delivery lost its rooted activation frame".to_string(),
                    )
                })?;
                if completed.request.id != request_id {
                    return Err(RuntimeExecutionError::Protocol(
                        "deferred delivery popped a different activation frame".to_string(),
                    ));
                }
                if let (
                    Some(replayed),
                    NativeTurnEvent::Coroutine {
                        thread,
                        response: _,
                    },
                ) = (replay_dead_thread, &event)
                    && replayed == *thread
                {
                    let error = StateArena::with_turn_state_mut(arena, current, |state| {
                        with_vm_context_gc(state, gc, string_pool, |state, gc| {
                            runtime_error_value(
                                state,
                                gc,
                                &RuntimeError::new("cannot resume dead coroutine"),
                            )
                        })
                    })
                    .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })?;
                    event = NativeTurnEvent::Coroutine {
                        thread: *thread,
                        response: ResumeResponse::Error(error),
                    };
                }
            }

            match event {
                NativeTurnEvent::AutomaticGc { stop_ci, saved_top } => {
                    if active_finalizer_role(activations, current).is_some() {
                        return Err(RuntimeExecutionError::Protocol(
                            "automatic GC checkpoint escaped from a finalizer".to_string(),
                        ));
                    }
                    let roots = RuntimeRootSet {
                        runtime_id: self.id,
                        main_handle: initial,
                        global_root: self.global_root,
                        registry_root: self.registry_root,
                        fixed_strings: &self.fixed_strings,
                    };
                    if gc.incremental_phase() == IncrementalPhase::Pause {
                        gc.begin_automatic_cycle();
                    }
                    let report = collect_incremental_step_at_safe_point(
                        roots,
                        // SAFETY: the VM returned at an audited instruction
                        // boundary and released the arena turn borrow.
                        unsafe { arena.as_mut() },
                        activations,
                        gc,
                        string_pool,
                        incremental_trace,
                        10_000,
                    )
                    .map_err(|error| {
                        *incremental_trace = None;
                        gc.abort_incremental_cycle();
                        RuntimeExecutionError::Protocol(format!(
                            "automatic incremental collection failed: {error}"
                        ))
                    })?;

                    if report.cycle_finished
                        && gc.pending_finalizer_count() != 0
                        && gc.begin_finalizer_drain()
                    {
                        activations.gc_frames.push(GcActivationFrame {
                            requester: current,
                            origin: GcActivationOrigin::Automatic { stop_ci, saved_top },
                            resume: driver_role,
                            draining_finalizers: true,
                            current_finalizer: None,
                            response: None,
                        });
                        if let Some(next) = next_finalizer_action(activations, gc)? {
                            action = next;
                            continue;
                        }
                        activations.gc_frames.pop();
                    }
                    action = NativeDriverAction::ResumeAutomaticGc {
                        resume: driver_role,
                    };
                    continue;
                }
                NativeTurnEvent::Request {
                    request,
                    caller_thread,
                } => {
                    let request = match *request {
                        RuntimeRequest::ProtectedCall(request) => {
                            let request = *request;
                            let deferred = request.deferred.as_ref().ok_or_else(|| {
                                RuntimeExecutionError::Protocol(
                                    "protected call reached Runtime without deferred metadata"
                                        .to_string(),
                                )
                            })?;
                            let role = NativeDriverRole::ProtectedCall {
                                stop_ci: deferred.saved_ci + 1,
                                // The fixed C++ pcall executes its protected
                                // target in-place over the native frame's first
                                // argument. Starting above the argument window
                                // leaves scratch objects in different GC-visible
                                // slots and shifts the observable Diff registers.
                                call_pos: deferred.func_pos + 1,
                                saved_top: deferred.snapshot.top(),
                            };
                            let parent_role = active_driver_role(activations, current);
                            let sequence = activations.allocate_callback_sequence()?;
                            let frame = ProtectedCallActivationFrame {
                                requester: current,
                                request: request.clone(),
                                phase: ProtectedCallPhase::Target,
                                role,
                                parent_role,
                                previous_allow_yield: deferred.snapshot.allow_yield(),
                                pending_response: None,
                                sequence,
                            };
                            activations.protected_calls.push(frame);
                            action = NativeDriverAction::StartProtectedCall {
                                function: request.function,
                                args: request.args,
                                role,
                            };
                            continue;
                        }
                        RuntimeRequest::CallbackHelper(request) => {
                            let request = *request;
                            let role = match &request.kind {
                                CallbackHelperKind::DebugHook {
                                    stop_ci,
                                    call_pos,
                                    saved_top,
                                    ..
                                }
                                | CallbackHelperKind::VmMetamethod {
                                    stop_ci,
                                    call_pos,
                                    saved_top,
                                    ..
                                } => NativeDriverRole::CallbackHelper {
                                    stop_ci: *stop_ci,
                                    call_pos: *call_pos,
                                    saved_top: *saved_top,
                                },
                                _ => {
                                    let deferred = request.deferred.as_ref().ok_or_else(|| {
                                        RuntimeExecutionError::Protocol(
                                            "callback helper reached Runtime without deferred metadata"
                                                .to_string(),
                                        )
                                    })?;
                                    NativeDriverRole::CallbackHelper {
                                        stop_ci: deferred.saved_ci + 1,
                                        call_pos: deferred.snapshot.top(),
                                        saved_top: deferred.snapshot.top(),
                                    }
                                }
                            };
                            let parent_role = active_driver_role(activations, current);
                            let sequence = activations.allocate_callback_sequence()?;
                            let frame = CallbackHelperActivationFrame {
                                requester: current,
                                request: request.clone(),
                                role,
                                parent_role,
                                pending_response: None,
                                sequence,
                            };
                            activations.callback_helpers.push(frame);
                            let wanted_results = callback_helper_wanted_results(&request.kind);
                            action = NativeDriverAction::StartCallbackHelper {
                                function: request.function,
                                args: request.args,
                                wanted_results,
                                role,
                            };
                            continue;
                        }
                        RuntimeRequest::FullCollection(request) => {
                            if request.deferred.is_none() {
                                return Err(RuntimeExecutionError::Protocol(
                                    "full collection request reached Runtime without deferred metadata"
                                        .to_string(),
                                ));
                            }
                            let resume = active_driver_role(activations, current);
                            activations.gc_frames.push(GcActivationFrame {
                                requester: current,
                                origin: GcActivationOrigin::Explicit(Box::new(request.clone())),
                                resume,
                                draining_finalizers: false,
                                current_finalizer: None,
                                response: None,
                            });
                            let roots = RuntimeRootSet {
                                runtime_id: self.id,
                                main_handle: initial,
                                global_root: self.global_root,
                                registry_root: self.registry_root,
                                fixed_strings: &self.fixed_strings,
                            };
                            let (response, cycle_completed) = match request.result {
                                crate::native::FullCollectionResult::Collect => {
                                    *incremental_trace = None;
                                    gc.abort_incremental_cycle();
                                    let collection = collect_full_stw_at_safe_point(
                                        roots,
                                        // SAFETY: the state turn ended before
                                        // this match and the arena remains
                                        // pinned for the session.
                                        unsafe { arena.as_mut() },
                                        activations,
                                        gc,
                                        string_pool,
                                    );
                                    if collection.is_ok() {
                                        // Pinned C++/Lua 5.1 compatibility:
                                        // a successful explicit full cycle
                                        // resumes automatic collection even
                                        // after a preceding `stop`.
                                        gc.restart_automatic();
                                    }
                                    let response = collection
                                        .map(|_| {
                                            CollectionResponse::Success(request.result.values(true))
                                        })
                                        .unwrap_or_else(|error| {
                                            CollectionResponse::ErrorMessage(error.to_string())
                                        });
                                    let completed =
                                        matches!(response, CollectionResponse::Success(_));
                                    (response, completed)
                                }
                                crate::native::FullCollectionResult::Step { size } => {
                                    match collect_incremental_step_at_safe_point(
                                        roots,
                                        // SAFETY: same scheduler safe point as
                                        // the full-collection branch.
                                        unsafe { arena.as_mut() },
                                        activations,
                                        gc,
                                        string_pool,
                                        incremental_trace,
                                        size,
                                    ) {
                                        Ok(report) => (
                                            CollectionResponse::Success(
                                                request.result.values(report.cycle_finished),
                                            ),
                                            report.cycle_finished,
                                        ),
                                        Err(error) => {
                                            *incremental_trace = None;
                                            gc.abort_incremental_cycle();
                                            (
                                                CollectionResponse::ErrorMessage(error.to_string()),
                                                false,
                                            )
                                        }
                                    }
                                }
                            };
                            if matches!(response, CollectionResponse::Success(_))
                                && cycle_completed
                                && !matches!(resume, NativeDriverRole::Finalizer { .. })
                                && gc.pending_finalizer_count() != 0
                                && gc.begin_finalizer_drain()
                            {
                                activations
                                    .gc_frames
                                    .last_mut()
                                    .expect("just-pushed collection frame")
                                    .draining_finalizers = true;
                                if let Some(next) = next_finalizer_action(activations, gc)? {
                                    action = next;
                                    continue;
                                }
                            }
                            let frame = activations
                                .gc_frames
                                .last_mut()
                                .expect("just-pushed collection frame");
                            frame.response = Some(response.clone());
                            action = NativeDriverAction::DeliverCollection {
                                request,
                                response,
                                resume,
                            };
                            continue;
                        }
                        RuntimeRequest::DebugUpvalue(request) => {
                            let request = *request;
                            if request.deferred.is_none() {
                                return Err(RuntimeExecutionError::Protocol(
                                    "debug Upvalue request reached Runtime without deferred metadata"
                                        .to_string(),
                                ));
                            }
                            if let Some(NativeDriverRole::Finalizer { stop_ci }) =
                                active_finalizer_role(activations, current)
                            {
                                action = NativeDriverAction::FailFinalizer {
                                    message:
                                        "cross-state debug Upvalue access is not supported from a __gc metamethod"
                                            .to_string(),
                                    stop_ci,
                                    saved_top: active_finalizer_saved_top(activations, current)
                                        .expect("active finalizer has a caller snapshot"),
                                };
                                continue;
                            }
                            if request.requester != current {
                                return Err(RuntimeExecutionError::Protocol(
                                    "debug Upvalue request named a different requester state"
                                        .to_string(),
                                ));
                            }
                            if request.owner == request.requester {
                                return Err(RuntimeExecutionError::Protocol(
                                    "local debug Upvalue request escaped into the Runtime scheduler"
                                        .to_string(),
                                ));
                            }

                            activations
                                .debug_upvalue_transfers
                                .push(DebugUpvalueTransferFrame {
                                    request: request.clone(),
                                    response: None,
                                });
                            if RuntimeFaultInjection::checkpoint(
                                runtime_fault_injection,
                                RuntimeFaultSite::UpvalueOwnerResolve,
                            ) {
                                return Err(RuntimeExecutionError::Protocol(
                                    "injected Runtime fault at UpvalueOwnerResolve".to_string(),
                                ));
                            }
                            let response = resolve_debug_upvalue_request(
                                arena,
                                gc,
                                string_pool,
                                runtime_id,
                                &request,
                            );
                            let transfer = activations
                                .debug_upvalue_transfers
                                .last_mut()
                                .ok_or_else(|| {
                                    RuntimeExecutionError::Protocol(
                                        "debug Upvalue access lost its rooted transfer frame"
                                            .to_string(),
                                    )
                                })?;
                            transfer.response = Some(response);
                            if RuntimeFaultInjection::checkpoint(
                                runtime_fault_injection,
                                RuntimeFaultSite::UpvalueResponseDelivery,
                            ) {
                                return Err(RuntimeExecutionError::Protocol(
                                    "injected Runtime fault at UpvalueResponseDelivery".to_string(),
                                ));
                            }
                            action = NativeDriverAction::DeliverDebugUpvalue {
                                frame: transfer.clone(),
                                resume: active_driver_role(activations, current),
                            };
                            current = request.requester;
                            continue;
                        }
                        RuntimeRequest::Resume(request) => request,
                    };
                    if let Some(NativeDriverRole::Finalizer { stop_ci }) =
                        active_finalizer_role(activations, current)
                    {
                        action = NativeDriverAction::FailFinalizer {
                            message: "coroutine.resume is not supported from a __gc metamethod"
                                .to_string(),
                            stop_ci,
                            saved_top: active_finalizer_saved_top(activations, current)
                                .expect("active finalizer has a caller snapshot"),
                        };
                        continue;
                    };
                    let target = request.target;
                    let frame = prepare_native_activation(gc, current, caller_thread, request)?;
                    let normal_deferred = if frame.previous_target_status == CoroutineStatus::Normal
                    {
                        activations
                            .frames
                            .iter()
                            .rev()
                            .find(|active| active.caller == target)
                            .and_then(|active| active.request.deferred.clone())
                    } else {
                        None
                    };
                    if frame.previous_target_status == CoroutineStatus::Normal
                        && normal_deferred.is_none()
                    {
                        return Err(RuntimeExecutionError::Protocol(
                            "Normal coroutine target has no suspended ancestor activation"
                                .to_string(),
                        ));
                    }
                    let request = frame.request.clone();
                    let previous_status = frame.previous_target_status;
                    activations.frames.push(frame);
                    action = NativeDriverAction::StartTarget {
                        request,
                        previous_status,
                        normal_deferred,
                    };
                    current = target;
                }
                NativeTurnEvent::UpvalueAccess(request) => {
                    if let Some(NativeDriverRole::Finalizer { stop_ci }) =
                        active_finalizer_role(activations, current)
                    {
                        action = NativeDriverAction::FailFinalizer {
                            message:
                                "cross-state open Upvalue access is not supported from a __gc metamethod"
                                    .to_string(),
                            stop_ci,
                            saved_top: active_finalizer_saved_top(activations, current)
                                .expect("active finalizer has a caller snapshot"),
                        };
                        continue;
                    }
                    let request = *request;
                    if request.requester != current {
                        return Err(RuntimeExecutionError::Protocol(
                            "open Upvalue request named a different requester state".to_string(),
                        ));
                    }
                    if request.owner == request.requester {
                        return Err(RuntimeExecutionError::Protocol(
                            "local open Upvalue escaped into the Runtime scheduler".to_string(),
                        ));
                    }

                    activations.upvalue_transfers.push(UpvalueTransferFrame {
                        request: request.clone(),
                        response: None,
                    });
                    if RuntimeFaultInjection::checkpoint(
                        runtime_fault_injection,
                        RuntimeFaultSite::UpvalueOwnerResolve,
                    ) {
                        return Err(RuntimeExecutionError::Protocol(
                            "injected Runtime fault at UpvalueOwnerResolve".to_string(),
                        ));
                    }
                    let response =
                        StateArena::with_turn_state_mut(arena, request.owner, |owner_state| {
                            with_vm_context_gc(owner_state, gc, string_pool, |owner_state, gc| {
                                execute_upvalue_owner_access(owner_state, gc, &request)
                            })
                        })
                        .map_err(|source| {
                            RuntimeAccessError::StateArena { runtime_id, source }
                        })??;
                    let transfer = activations.upvalue_transfers.last_mut().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "open Upvalue access lost its rooted transfer frame".to_string(),
                        )
                    })?;
                    transfer.response = Some(response);
                    if RuntimeFaultInjection::checkpoint(
                        runtime_fault_injection,
                        RuntimeFaultSite::UpvalueResponseDelivery,
                    ) {
                        return Err(RuntimeExecutionError::Protocol(
                            "injected Runtime fault at UpvalueResponseDelivery".to_string(),
                        ));
                    }
                    action = NativeDriverAction::DeliverUpvalue {
                        frame: transfer.clone(),
                        resume: active_driver_role(activations, current),
                    };
                    current = request.requester;
                }
                NativeTurnEvent::Coroutine { thread, response } => {
                    let frame = activations.frames.last_mut().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "coroutine stopped without an activation frame".to_string(),
                        )
                    })?;
                    if frame.request.thread != thread {
                        return Err(RuntimeExecutionError::Protocol(
                            "stopped coroutine does not match activation target".to_string(),
                        ));
                    }
                    if RuntimeFaultInjection::checkpoint(
                        runtime_fault_injection,
                        RuntimeFaultSite::ActivationUnwind,
                    ) {
                        return Err(RuntimeExecutionError::Protocol(
                            "injected Runtime fault at ActivationUnwind".to_string(),
                        ));
                    }
                    restore_native_activation_links(gc, frame)?;
                    let replay_dead_ancestor = match frame.caller_thread {
                        Some(caller) => {
                            gc.with_ref(caller, Thread::status).map_err(|error| {
                                RuntimeExecutionError::Protocol(format!(
                                    "invalid caller Thread: {error}"
                                ))
                            })? == CoroutineStatus::Dead
                        }
                        None => false,
                    };
                    if replay_dead_ancestor && let Some(caller) = frame.caller_thread {
                        gc.with_mut(caller, |thread| {
                            thread.set_status(CoroutineStatus::Running);
                        })
                        .map_err(|error| {
                            RuntimeExecutionError::Protocol(format!(
                                "invalid replay caller Thread: {error}"
                            ))
                        })?;
                    }
                    frame.pending_response = Some(response.clone());
                    frame.replay_dead_ancestor = replay_dead_ancestor;
                    let delivery_frame = frame.clone();
                    current = frame.caller;
                    let resume = active_driver_role(activations, current);
                    action = NativeDriverAction::Deliver {
                        frame: delivery_frame,
                        response,
                        replay_dead_ancestor,
                        resume,
                    };
                }
                NativeTurnEvent::ProtectedCall(response) => {
                    let frame = activations.protected_calls.last_mut().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "protected helper stopped without an activation frame".to_string(),
                        )
                    })?;
                    if frame.requester != current {
                        return Err(RuntimeExecutionError::Protocol(
                            "protected helper stopped on a different requester state".to_string(),
                        ));
                    }
                    match (frame.request.kind, frame.phase, &response) {
                        (
                            ProtectedCallKind::XPCall,
                            ProtectedCallPhase::Target,
                            ResumeResponse::Error(error),
                        ) => {
                            frame.phase = ProtectedCallPhase::Handler;
                            frame.pending_response = Some(response.clone());
                            let handler = frame.request.handler.clone().ok_or_else(|| {
                                RuntimeExecutionError::Protocol(
                                    "xpcall activation lost its error handler".to_string(),
                                )
                            })?;
                            action = NativeDriverAction::StartProtectedCall {
                                function: handler,
                                args: vec![error.clone()],
                                role: frame.role,
                            };
                        }
                        _ => {
                            let response = match (frame.phase, response) {
                                (ProtectedCallPhase::Target, ResumeResponse::Success(values)) => {
                                    let mut protected = Vec::with_capacity(values.len() + 1);
                                    protected.push(Value::Boolean(true));
                                    protected.extend(values);
                                    ResumeResponse::Success(protected)
                                }
                                (ProtectedCallPhase::Target, ResumeResponse::Error(error))
                                | (ProtectedCallPhase::Handler, ResumeResponse::Error(error)) => {
                                    ResumeResponse::Error(error)
                                }
                                (ProtectedCallPhase::Handler, ResumeResponse::Success(values)) => {
                                    ResumeResponse::Success(values)
                                }
                            };
                            frame.pending_response = Some(response.clone());
                            action = NativeDriverAction::FinishProtectedCall {
                                frame: frame.clone(),
                                response,
                            };
                        }
                    }
                }
                NativeTurnEvent::CallbackHelper(response) => {
                    let frame = activations.callback_helpers.last_mut().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "callback helper stopped without an activation frame".to_string(),
                        )
                    })?;
                    if frame.requester != current {
                        return Err(RuntimeExecutionError::Protocol(
                            "callback helper stopped on a different requester state".to_string(),
                        ));
                    }
                    frame.pending_response = Some(response.clone());
                    let mut next_callback = None;
                    let continue_helper = match (&mut frame.request.kind, &response) {
                        (
                            CallbackHelperKind::LoadReader {
                                source,
                                terminal_error,
                                ..
                            },
                            ResumeResponse::Success(values),
                        ) => match values.first().unwrap_or(&Value::Nil) {
                            Value::String(chunk) => {
                                let chunk = gc.with_string_bytes(*chunk, <[u8]>::to_vec).map_err(
                                    |error| {
                                        RuntimeExecutionError::Protocol(format!(
                                            "load reader returned an invalid string: {error}"
                                        ))
                                    },
                                )?;
                                if chunk.is_empty() {
                                    false
                                } else if source.try_reserve(chunk.len()).is_err() {
                                    *terminal_error = Some(
                                        "not enough memory while reading load source".to_string(),
                                    );
                                    false
                                } else {
                                    source.extend_from_slice(&chunk);
                                    if source.len() >= 2
                                        && let Some(message) = definite_load_syntax_error(source)
                                    {
                                        *terminal_error = Some(message);
                                        false
                                    } else {
                                        true
                                    }
                                }
                            }
                            Value::Nil
                            | Value::Boolean(_)
                            | Value::Number(_)
                            | Value::Table(_)
                            | Value::Function(_)
                            | Value::Userdata(_)
                            | Value::Thread(_)
                            | Value::LightUserdata(_) => false,
                        },
                        (
                            CallbackHelperKind::TableSort {
                                state,
                                terminal_error,
                            },
                            ResumeResponse::Success(values),
                        ) => {
                            let result = values.first().is_some_and(|value| !value.is_false());
                            match advance_table_sort_state(gc, state, Some(result))? {
                                TableSortAdvance::Callback { function, args } => {
                                    next_callback = Some((function, args));
                                    true
                                }
                                TableSortAdvance::Complete => {
                                    if let Err(error) = commit_table_sort(gc, state) {
                                        *terminal_error = Some(error.message);
                                    }
                                    false
                                }
                                TableSortAdvance::Error(message) => {
                                    *terminal_error = Some(message);
                                    false
                                }
                            }
                        }
                        (
                            CallbackHelperKind::TableForeach { state },
                            ResumeResponse::Success(values),
                        ) if values.first().is_none_or(Value::is_nil) => {
                            match advance_table_foreach_state(gc, state)? {
                                TableForeachAdvance::Callback { function, args } => {
                                    next_callback = Some((function, args));
                                    true
                                }
                                TableForeachAdvance::Complete => false,
                            }
                        }
                        (
                            CallbackHelperKind::PackageModuleOptions {
                                state,
                                terminal_error,
                            },
                            ResumeResponse::Success(_),
                        ) => {
                            if let Some(option) = state.options.get(state.next).cloned() {
                                state.next = state.next.saturating_add(1);
                                if matches!(option, Value::Function(_)) {
                                    next_callback =
                                        Some((option, vec![Value::Table(state.module)]));
                                    true
                                } else {
                                    *terminal_error =
                                        Some("module option must be a function".to_string());
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        (
                            CallbackHelperKind::Gsub {
                                state,
                                terminal_error,
                            },
                            ResumeResponse::Success(values),
                        ) => {
                            let value = values.first().cloned().unwrap_or(Value::Nil);
                            match advance_gsub_state(gc, state, Some(value))? {
                                GsubAdvance::Callback { function, args } => {
                                    next_callback = Some((function, args));
                                    true
                                }
                                GsubAdvance::Complete => false,
                                GsubAdvance::Error(message) => {
                                    *terminal_error = Some(message);
                                    false
                                }
                            }
                        }
                        (CallbackHelperKind::ToString, _)
                        | (CallbackHelperKind::DoFile, _)
                        | (CallbackHelperKind::LoadReader { .. }, ResumeResponse::Error(_))
                        | (CallbackHelperKind::TableSort { .. }, ResumeResponse::Error(_))
                        | (CallbackHelperKind::TableForeach { .. }, _)
                        | (CallbackHelperKind::PackageRequire { .. }, _)
                        | (
                            CallbackHelperKind::PackageModuleOptions { .. },
                            ResumeResponse::Error(_),
                        )
                        | (CallbackHelperKind::Gsub { .. }, ResumeResponse::Error(_))
                        | (CallbackHelperKind::DebugHook { .. }, _)
                        | (CallbackHelperKind::VmMetamethod { .. }, _) => false,
                    };
                    if continue_helper {
                        if let Some((function, args)) = next_callback {
                            frame.request.function = function;
                            frame.request.args = args;
                        }
                        frame.pending_response = None;
                        let wanted_results = callback_helper_wanted_results(&frame.request.kind);
                        action = NativeDriverAction::StartCallbackHelper {
                            function: frame.request.function.clone(),
                            args: frame.request.args.clone(),
                            wanted_results,
                            role: frame.role,
                        };
                    } else {
                        action = NativeDriverAction::FinishCallbackHelper {
                            frame: frame.clone(),
                            response,
                        };
                    }
                }
                NativeTurnEvent::Finalizer(result) => {
                    let frame = activations.gc_frames.last().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "finalizer stopped without a collection activation".to_string(),
                        )
                    })?;
                    if !frame.draining_finalizers || frame.requester != current {
                        return Err(RuntimeExecutionError::Protocol(
                            "finalizer stopped outside its owning collection activation"
                                .to_string(),
                        ));
                    }
                    let (userdata, _) = frame.current_finalizer.clone().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "finalizer completion lost its pending userdata".to_string(),
                        )
                    })?;
                    let callback_top = frame.saved_top().ok_or_else(|| {
                        RuntimeExecutionError::Protocol(
                            "finalizer activation lost its caller snapshot".to_string(),
                        )
                    })?;
                    let automatic_failure =
                        matches!(&frame.origin, GcActivationOrigin::Automatic { .. })
                            .then(|| result.as_ref().err().cloned())
                            .flatten();
                    let callback_error = StateArena::with_turn_state_mut(arena, current, |state| {
                        with_vm_context_gc(state, gc, string_pool, |state, gc| {
                            let error =
                                (!matches!(&frame.origin, GcActivationOrigin::Automatic { .. }))
                                    .then(|| {
                                        result
                                            .as_ref()
                                            .err()
                                            .map(|error| runtime_error_value(state, gc, error))
                                    })
                                    .flatten();
                            let scratch_start = state.current_call_info().top;
                            let initialized = state.stack.size();
                            for index in scratch_start..initialized {
                                if let Some(slot) = state.stack.at_mut(index) {
                                    *slot = Value::Nil;
                                }
                            }
                            state.stack.set_top(scratch_start);
                            state.top = callback_top;
                            error
                        })
                    })
                    .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })?;
                    if !gc.acknowledge_pending_finalizer(userdata) {
                        return Err(RuntimeExecutionError::Protocol(
                            "finalizer acknowledgement lost its pending userdata".to_string(),
                        ));
                    }
                    let frame = activations
                        .gc_frames
                        .last_mut()
                        .expect("validated collection activation remains");
                    frame.current_finalizer = None;
                    if let Some(error) = automatic_failure {
                        gc.end_finalizer_drain();
                        activations.gc_frames.pop();
                        return Err(RuntimeExecutionError::Vm(error));
                    } else if let Some(error) = callback_error {
                        gc.end_finalizer_drain();
                        frame.draining_finalizers = false;
                        let response = CollectionResponse::ErrorValue(error);
                        frame.response = Some(response.clone());
                        let request = frame
                            .explicit_request()
                            .expect("explicit finalizer error owns a collection request")
                            .clone();
                        let resume = frame.resume;
                        action = NativeDriverAction::DeliverCollection {
                            request,
                            response,
                            resume,
                        };
                    } else if let Some(next) = next_finalizer_action(activations, gc)? {
                        action = next;
                    } else {
                        let frame = activations
                            .gc_frames
                            .last_mut()
                            .expect("collection activation remains after drain");
                        if let Some(request) = frame.explicit_request().cloned() {
                            let response = CollectionResponse::Success(request.result.values(true));
                            frame.response = Some(response.clone());
                            let resume = frame.resume;
                            action = NativeDriverAction::DeliverCollection {
                                request,
                                response,
                                resume,
                            };
                        } else {
                            let resume = frame.automatic_resume().ok_or_else(|| {
                                RuntimeExecutionError::Protocol(
                                    "automatic GC activation lost its resume role".to_string(),
                                )
                            })?;
                            activations.gc_frames.pop();
                            action = NativeDriverAction::ResumeAutomaticGc { resume };
                        }
                    }
                }
                NativeTurnEvent::Main(result) => {
                    if !activations.frames.is_empty()
                        || !activations.upvalue_transfers.is_empty()
                        || !activations.debug_upvalue_transfers.is_empty()
                        || !activations.protected_calls.is_empty()
                        || !activations.gc_frames.is_empty()
                    {
                        return Err(RuntimeExecutionError::Protocol(
                            "main state stopped while coroutine activations remain".to_string(),
                        ));
                    }
                    return result.map_err(RuntimeExecutionError::Vm);
                }
            }
        }
    }

    /// Execute a top-level Proto with Lua varargs and consume results while
    /// they remain owned by the main-state stack.
    ///
    /// Preparation and result collection occur outside the scheduler session;
    /// the middle execution always uses the same Runtime-owned trampoline as
    /// [`Runtime::execute_proto`].
    pub fn execute_proto_with_args(
        &mut self,
        proto: GcRef<Proto>,
        args: Vec<Value>,
        consume_results: impl FnOnce(&LuaState, &[Value]),
    ) -> Result<(), RuntimeExecutionError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            }
            .into());
        }
        if self.active_executions != 0 {
            return Err(RuntimeAccessError::ActiveExecutions {
                runtime_id: self.id,
                count: self.active_executions,
            }
            .into());
        }
        self.with_main_state_mut(|state| {
            state.unwind_call_info_to(0);
            state.current_call_info_mut().reset();
            state.current_call_info_mut().varargs = args;
            state.top = 0;
            state.status = crate::state::ThreadStatus::Ok;
        })?;

        let execution = self.execute_proto(proto);
        let mut parts = self.parts_mut()?;
        parts.with_context(|state| {
            let results = execution.is_ok().then(|| state_stack_values(state));
            if let Err(RuntimeExecutionError::Vm(error)) = &execution {
                emit_runtime_error_trace(state, &error.message);
            }
            let reset = TopLevelStateResetGuard { state };
            execution?;
            consume_results(
                &*reset.state,
                results
                    .as_deref()
                    .expect("successful execution exposes its result buffer"),
            );
            Ok(())
        })
    }

    /// Deterministically reclaim Runtime-owned Rust states and GC allocations.
    ///
    /// Close is idempotent after the first successful call. It drains pending
    /// and newly allocated Lua `__gc` callbacks while isolating callback
    /// errors, closes validated open Upvalues, invalidates every StateArena
    /// slot, destroys non-fixed Threads before other non-fixed objects and
    /// fixed objects last, and empties the live StringPool. Weakly registered
    /// Runtime resources are explicitly drained after Lua finalizers and
    /// before any state allocation is destroyed.
    pub fn close(&mut self) -> Result<RuntimeCloseReport, RuntimeAccessError> {
        self.check_owner()?;
        if self.active_executions != 0 {
            return Err(RuntimeAccessError::ActiveExecutions {
                runtime_id: self.id,
                count: self.active_executions,
            });
        }

        self.close_owner_inactive()
    }

    fn close_owner_inactive(&mut self) -> Result<RuntimeCloseReport, RuntimeAccessError> {
        match self.phase {
            RuntimePhase::Closed => Ok(self.close_report(true)),
            RuntimePhase::Closing => Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            }),
            RuntimePhase::Running => {
                self.validate_shutdown_state().map_err(|source| {
                    RuntimeAccessError::StateArena {
                        runtime_id: self.id,
                        source,
                    }
                })?;
                if RuntimeFaultInjection::checkpoint(
                    &mut self.runtime_fault_injection,
                    RuntimeFaultSite::ShutdownPreflight,
                ) {
                    return Err(RuntimeAccessError::InjectedFault {
                        runtime_id: self.id,
                        site: RuntimeFaultSite::ShutdownPreflight,
                    });
                }
                let close_finalizers = self.drain_finalizers_for_close();
                let close_resources = self.drain_runtime_resources_for_close();
                self.heap
                    .as_ref()
                    .get_ref()
                    .state_arena()
                    .validate_runtime_resource_drain_complete()
                    .map_err(|source| RuntimeAccessError::StateArena {
                        runtime_id: self.id,
                        source,
                    })?;
                self.validate_shutdown_state().map_err(|source| {
                    RuntimeAccessError::StateArena {
                        runtime_id: self.id,
                        source,
                    }
                })?;
                self.phase = RuntimePhase::Closing;
                let summary = match self.shutdown_contents(close_finalizers, close_resources) {
                    Ok(summary) => summary,
                    Err(source) => {
                        // All fallible arena ownership checks are preflighted
                        // before Closing. Retain the phase to fail closed if an
                        // internal invariant nevertheless changes.
                        return Err(RuntimeAccessError::StateArena {
                            runtime_id: self.id,
                            source,
                        });
                    }
                };
                self.shutdown_summary = Some(summary);
                self.phase = RuntimePhase::Closed;
                Ok(self.close_report(false))
            }
        }
    }

    fn drain_finalizers_for_close(&mut self) -> (usize, usize) {
        let Ok(mut parts) = self.parts_mut() else {
            return (0, 0);
        };
        let (state, gc, strings) = parts.split_mut();
        gc.prepare_all_finalizable_userdata();
        if gc.pending_finalizer_count() == 0 || !gc.begin_finalizer_drain() {
            return (0, 0);
        }

        let mut attempted = 0usize;
        let mut errors = 0usize;
        while let Some(userdata) = gc.next_pending_finalizer() {
            let callback = gc.pending_finalizer_callback(userdata);
            if let Some(callback) = callback {
                attempted = attempted.saturating_add(1);
                let result = with_vm_context_gc(state, gc, strings, |state, gc| {
                    call_value_with_results(
                        state,
                        gc,
                        callback,
                        &[Value::Userdata(userdata)],
                        Some(0),
                        |_, _, _| (),
                    )
                });
                if result.is_err() {
                    errors = errors.saturating_add(1);
                }
            }
            let _ = gc.acknowledge_pending_finalizer(userdata);
            // A close callback may allocate another finalizable userdata.
            gc.prepare_all_finalizable_userdata();
        }
        gc.end_finalizer_drain();
        (attempted, errors)
    }

    fn drain_runtime_resources_for_close(&mut self) -> RuntimeResourceDrainReport {
        const MAX_CLOSE_RESOURCE_HOOKS: usize = 1_000_000;

        let mut report = RuntimeResourceDrainReport::default();
        while report.attempted < MAX_CLOSE_RESOURCE_HOOKS {
            let entry = {
                // SAFETY: close has exclusive owner-thread Runtime access.
                let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
                // SAFETY: no StateArena reference is retained while the hook
                // runs through a scoped main-state borrow below.
                unsafe { &mut *heap.state_arena.get() }.next_runtime_resource()
            };
            let Some(entry) = entry else {
                break;
            };

            report.attempted = report.attempted.saturating_add(1);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut parts = self
                    .parts_mut()
                    .expect("shutdown preflight validated the main-state service turn");
                parts.with_context_gc(|state, gc| (entry.drain)(state, gc, entry.object_id))
            }));
            match outcome {
                Ok(RuntimeResourceDrainOutcome::Drained) => {
                    report.drained = report.drained.saturating_add(1);
                }
                Ok(RuntimeResourceDrainOutcome::AlreadyClosed) => {
                    report.already_closed = report.already_closed.saturating_add(1);
                }
                Ok(RuntimeResourceDrainOutcome::Stale) => {
                    report.stale = report.stale.saturating_add(1);
                }
                Ok(RuntimeResourceDrainOutcome::Failed) | Err(_) => {
                    report.errors = report.errors.saturating_add(1);
                }
            }
        }

        // A hostile hook could continually append distinct registrations.
        // Bound the drain, report discarded entries as errors, and release
        // registry capacity before state/heap teardown regardless.
        let pending = {
            // SAFETY: same exclusive close access as above.
            let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
            // SAFETY: no StateArena state borrow is active here.
            let arena = unsafe { &mut *heap.state_arena.get() };
            let pending = arena.pending_runtime_resource_count();
            arena.finish_runtime_resource_drain();
            pending
        };
        report.errors = report.errors.saturating_add(pending);
        report
    }

    fn validate_shutdown_state(&self) -> Result<(), StateResolveError> {
        let handle = self
            .main_state_handle
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let heap = self.heap.as_ref().get_ref();
        if !heap.native_activations.frames.is_empty()
            || !heap.native_activations.upvalue_transfers.is_empty()
            || !heap.native_activations.debug_upvalue_transfers.is_empty()
            || !heap.native_activations.protected_calls.is_empty()
            || !heap.native_activations.callback_helpers.is_empty()
            || !heap.native_activations.gc_frames.is_empty()
        {
            return Err(StateResolveError::ArenaInvariant {
                reason: "native activation stack is not empty at shutdown",
            });
        }
        heap.state_arena().validate_main(handle)?;
        heap.state_arena().validate_owned_drain()
    }

    fn shutdown_contents(
        &mut self,
        close_finalizers: (usize, usize),
        close_resources: RuntimeResourceDrainReport,
    ) -> Result<RuntimeShutdownSummary, StateResolveError> {
        let arena_report = {
            // SAFETY: RuntimeStorage remains pinned and shutdown has exclusive
            // owner-thread access with no execution guard.
            let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
            // SAFETY: shutdown has exclusive Runtime access and the arena and
            // collector are disjoint RuntimeStorage fields.
            unsafe { &mut *heap.state_arena.get() }.drain_owned(heap.heap.collector_mut())?
        };

        debug_assert_eq!(arena_report.drained_main_states, 1);
        self.main_state_handle = None;
        let state_shutdown = arena_report.state_shutdown;

        let global_root = self.global_root.take();
        let registry_root = self.registry_root.take();
        self.fixed_strings.clear();
        // SAFETY: same pinned/exclusive shutdown invariant as above. All state
        // allocations have been detached and no VM service context remains
        // active before collector allocations are destroyed.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        if let Some(global_root) = global_root {
            heap.heap.collector_mut().remove_root(global_root);
        }
        if let Some(registry_root) = registry_root {
            heap.heap.collector_mut().remove_root(registry_root);
        }
        let gc_report = heap.heap.destroy_all();

        Ok(RuntimeShutdownSummary {
            drained_coroutine_states: arena_report.drained_coroutine_states,
            state_shutdown,
            destroyed_objects: gc_report.destroyed_objects(),
            destroyed_threads: gc_report.destroyed_threads,
            destroyed_fixed_objects: gc_report.destroyed_fixed,
            pending_lua_finalizers_discarded: gc_report.pending_finalizers_discarded,
            lua_finalizers_attempted: close_finalizers.0,
            lua_finalizer_errors_ignored: close_finalizers.1,
            resource_drain: close_resources,
        })
    }

    fn close_report(&self, already_closed: bool) -> RuntimeCloseReport {
        let heap = self.heap.as_ref().get_ref();
        let remaining_objects = heap.heap.object_count();
        let remaining_coroutine_states = heap.state_arena().live_coroutine_state_count();
        let remaining_temporary_state_roots = heap.state_arena().temporary_state_root_count();
        let remaining_collector_queue_entries = heap.heap.collector().transient_queue_entry_count();
        let allocator = heap.heap.allocator_stats();
        let summary = self.shutdown_summary.unwrap_or_default();
        RuntimeCloseReport {
            runtime_id: self.id,
            already_closed,
            heap_reclamation_deferred: remaining_objects != 0
                || heap.heap.collector().root_count() != 0
                || !heap.heap.strings().is_empty()
                || remaining_coroutine_states != 0
                || remaining_temporary_state_roots != 0
                || remaining_collector_queue_entries != 0
                || allocator.live_bytes != 0,
            remaining_objects,
            remaining_roots: heap.heap.collector().root_count(),
            remaining_interned_strings: heap.heap.interned_string_count(),
            remaining_estimated_bytes: heap.heap.accounted_bytes(),
            remaining_allocator_live_bytes: allocator.live_bytes,
            allocator_peak_bytes: allocator.peak_bytes,
            allocator_total_allocated_bytes: allocator.total_allocated_bytes,
            remaining_coroutine_states,
            remaining_temporary_state_roots,
            rejected_temporary_state_root_releases: heap
                .state_arena()
                .rejected_temporary_root_releases,
            remaining_collector_queue_entries,
            drained_coroutine_states: summary.drained_coroutine_states,
            closed_open_upvalues: summary.state_shutdown.closed_open_upvalues,
            rejected_open_upvalue_edges: summary.state_shutdown.rejected_open_upvalue_edges,
            open_upvalue_owner_mismatches: summary.state_shutdown.open_upvalue_owner_mismatches,
            open_upvalue_stack_values_missing: summary
                .state_shutdown
                .open_upvalue_stack_values_missing,
            destroyed_objects: summary.destroyed_objects,
            destroyed_threads: summary.destroyed_threads,
            destroyed_fixed_objects: summary.destroyed_fixed_objects,
            pending_lua_finalizers_discarded: summary.pending_lua_finalizers_discarded,
            lua_finalizers_attempted: summary.lua_finalizers_attempted,
            lua_finalizer_errors_ignored: summary.lua_finalizer_errors_ignored,
            runtime_resources_attempted: summary.resource_drain.attempted,
            runtime_resources_drained: summary.resource_drain.drained,
            runtime_resources_already_closed: summary.resource_drain.already_closed,
            runtime_resources_stale: summary.resource_drain.stale,
            runtime_resource_drain_errors: summary.resource_drain.errors,
            lua_gc_callback_debt: false,
            io_resource_drain_debt: false,
        }
    }

    fn close_for_drop(&mut self) {
        if self.phase == RuntimePhase::Closed {
            return;
        }

        if thread::current().id() == self.owner_thread
            && self.active_executions == 0
            && self.close_owner_inactive().is_ok()
        {
            return;
        }

        // Safe Rust cannot move Runtime across threads or enter Drop while a
        // RuntimePartsMut borrow exists. If unsafe host code violates those
        // invariants, avoid cross-thread callbacks; StateArena::Drop releases
        // every state Box and Heap::Drop reclaims objects without Lua hooks.
        self.main_state_handle = None;
        self.global_root.take();
        self.registry_root.take();
        self.fixed_strings.clear();
        self.phase = RuntimePhase::Closed;
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // A RuntimePartsMut guard exclusively borrows the Runtime, so safe Rust
        // cannot enter Drop while an execution is active. The normal
        // owner-thread path delegates to the same deterministic teardown used
        // by explicit close.
        self.close_for_drop();
    }
}

fn active_finalizer_role(
    activations: &NativeActivationStack,
    requester: StateHandle,
) -> Option<NativeDriverRole> {
    activations.gc_frames.iter().rev().find_map(|frame| {
        if !frame.draining_finalizers || frame.requester != requester {
            return None;
        }
        frame
            .stop_ci()
            .map(|stop_ci| NativeDriverRole::Finalizer { stop_ci })
    })
}

fn active_driver_role(
    activations: &NativeActivationStack,
    requester: StateHandle,
) -> NativeDriverRole {
    let protected =
        activations.protected_calls.iter().rev().find_map(|frame| {
            (frame.requester == requester).then_some((frame.sequence, frame.role))
        });
    let helper =
        activations.callback_helpers.iter().rev().find_map(|frame| {
            (frame.requester == requester).then_some((frame.sequence, frame.role))
        });
    match (protected, helper) {
        (Some(left), Some(right)) => Some(if left.0 > right.0 { left.1 } else { right.1 }),
        (Some((_, role)), None) | (None, Some((_, role))) => Some(role),
        (None, None) => None,
    }
    .or_else(|| active_finalizer_role(activations, requester))
    .unwrap_or(NativeDriverRole::Program)
}

fn active_finalizer_saved_top(
    activations: &NativeActivationStack,
    requester: StateHandle,
) -> Option<usize> {
    activations.gc_frames.iter().rev().find_map(|frame| {
        (frame.draining_finalizers && frame.requester == requester)
            .then(|| frame.saved_top())
            .flatten()
    })
}

fn next_finalizer_action(
    activations: &mut NativeActivationStack,
    gc: &mut GarbageCollector,
) -> Result<Option<NativeDriverAction>, RuntimeExecutionError> {
    loop {
        let frame = activations.gc_frames.last_mut().ok_or_else(|| {
            RuntimeExecutionError::Protocol(
                "finalizer drain has no collection activation".to_string(),
            )
        })?;
        if !frame.draining_finalizers {
            return Err(RuntimeExecutionError::Protocol(
                "finalizer scheduling requested outside a drain".to_string(),
            ));
        }
        let Some(userdata) = gc.next_pending_finalizer() else {
            gc.end_finalizer_drain();
            frame.draining_finalizers = false;
            frame.current_finalizer = None;
            return Ok(None);
        };
        let Some(callback) = gc.pending_finalizer_callback(userdata) else {
            if !gc.acknowledge_pending_finalizer(userdata) {
                return Err(RuntimeExecutionError::Protocol(
                    "missing finalizer callback could not be acknowledged".to_string(),
                ));
            }
            continue;
        };
        let stop_ci = frame.stop_ci().ok_or_else(|| {
            RuntimeExecutionError::Protocol(
                "finalizer scheduling lost its caller frame".to_string(),
            )
        })?;
        frame.current_finalizer = Some((userdata, callback.clone()));
        return Ok(Some(NativeDriverAction::StartFinalizer {
            callback,
            userdata,
            stop_ci,
        }));
    }
}

fn execute_native_driver_action(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    action: NativeDriverAction,
) -> Result<VmExit, RuntimeError> {
    match action {
        NativeDriverAction::StartProto(proto) => execute_proto(state, proto, gc),
        NativeDriverAction::StartTarget {
            request,
            previous_status,
            normal_deferred,
        } => {
            if state.current_thread != Some(request.thread) {
                return Err(RuntimeError::new(
                    "Runtime-native target state does not own the requested Thread",
                ));
            }
            state.allow_yield = state
                .allow_yield
                .checked_add(1)
                .ok_or_else(|| RuntimeError::new("coroutine yield permission overflow"))?;

            let first_resume = gc
                .with_ref(request.thread, Thread::is_first_resume)
                .map_err(|error| RuntimeError::new(format!("invalid coroutine Thread: {error}")))?;
            if first_resume {
                install_initial_resume_args(state, &request.args);
                start_lua_call_at_stack(state, gc, 0, request.args.len(), None)?;
                gc.with_mut(request.thread, |thread| {
                    thread.mark_resumed();
                    thread.set_saved_nexeccalls(state.nccalls);
                })
                .map_err(|error| RuntimeError::new(format!("invalid coroutine Thread: {error}")))?;
                if let Some(request_id) = dispatch_started_lua_call_hook(state, gc)? {
                    return Ok(VmExit::NativeRequest(request_id));
                }
                if let Some(proto) = state.current_call_info().proto {
                    let pc = state.current_call_info().savedpc.unwrap_or(0);
                    emit_coroutine_resume_trace(state, gc, proto, pc);
                }
                resume_lua_thread(state, gc)
            } else if previous_status == CoroutineStatus::Normal {
                let deferred = normal_deferred.ok_or_else(|| {
                    RuntimeError::new(
                        "Normal coroutine re-entry has no deferred ancestor continuation",
                    )
                })?;
                let completion = finish_deferred_native_values(state, gc, &deferred, request.args)?;
                if let Some(proto) = state.current_call_info().proto {
                    let pc = state.current_call_info().savedpc.unwrap_or(0);
                    emit_coroutine_resume_trace(state, gc, proto, pc);
                }
                resume_after_deferred_completion(
                    state,
                    gc,
                    &deferred,
                    NativeDriverRole::Program,
                    completion,
                )
            } else {
                install_resume_args(state, &request.args);
                record_trace_nil_range(state, state.top, state.current_call_info().top);
                if let Some(proto) = state.current_call_info().proto {
                    let pc = state
                        .current_call_info()
                        .savedpc
                        .unwrap_or(0)
                        .saturating_add(1);
                    emit_coroutine_resume_trace(state, gc, proto, pc);
                }
                resume_lua_thread(state, gc)
            }
        }
        NativeDriverAction::Deliver {
            frame,
            response,
            replay_dead_ancestor,
            resume,
        } => {
            let deferred = frame.request.deferred.as_ref().ok_or_else(|| {
                RuntimeError::new("Runtime-native activation lost its deferred call")
            })?;
            let completion = if replay_dead_ancestor {
                deferred.snapshot.restore(state);
                finish_deferred_native_values(state, gc, deferred, Vec::new())?
            } else {
                finish_deferred_native_call(state, gc, deferred, frame.request.envelope, response)?
            };
            resume_after_deferred_completion(state, gc, deferred, resume, completion)
        }
        NativeDriverAction::StartProtectedCall {
            function,
            args,
            role:
                NativeDriverRole::ProtectedCall {
                    stop_ci, call_pos, ..
                },
        } => {
            // Runtime-owned pcall/xpcall targets are non-resumable boundaries.
            // Keep the barrier on the LuaState itself so it remains effective
            // while nested Runtime requests switch away from and back to this
            // state. The activation frame restores the previous permission.
            state.allow_yield = 0;
            start_runtime_callback_target(state, gc, function, &args, stop_ci, call_pos, None)
        }
        NativeDriverAction::StartProtectedCall { .. } => Err(RuntimeError::new(
            "VM: protected helper action lost its protected role",
        )),
        NativeDriverAction::FinishProtectedCall { frame, response } => {
            // Restore before completing the deferred native call: completion can
            // immediately resume an enclosing callback/coroutine and must observe
            // the permission that was active outside this protected boundary.
            state.allow_yield = frame.previous_allow_yield;
            let deferred = frame.request.deferred.as_ref().ok_or_else(|| {
                RuntimeError::new("protected helper activation lost its deferred call")
            })?;
            let values = match (frame.phase, response) {
                (ProtectedCallPhase::Target, ResumeResponse::Success(values)) => values,
                (ProtectedCallPhase::Target, ResumeResponse::Error(error)) => {
                    vec![Value::Boolean(false), error]
                }
                (ProtectedCallPhase::Handler, ResumeResponse::Success(values)) => {
                    let mut protected = Vec::with_capacity(values.len() + 1);
                    protected.push(Value::Boolean(false));
                    protected.extend(values);
                    protected
                }
                (ProtectedCallPhase::Handler, ResumeResponse::Error(error)) => {
                    let error = if error.is_nil() {
                        runtime_error_value(
                            state,
                            gc,
                            &RuntimeError::new("error in error handling"),
                        )
                    } else {
                        error
                    };
                    vec![Value::Boolean(false), error]
                }
            };
            let completion = finish_deferred_protected_values(state, gc, deferred, values)?;
            resume_after_deferred_completion(state, gc, deferred, frame.parent_role, completion)
        }
        NativeDriverAction::StartCallbackHelper {
            function,
            args,
            wanted_results,
            role:
                NativeDriverRole::CallbackHelper {
                    stop_ci, call_pos, ..
                },
        } => start_runtime_callback_target(
            state,
            gc,
            function,
            &args,
            stop_ci,
            call_pos,
            wanted_results,
        ),
        NativeDriverAction::StartCallbackHelper { .. } => Err(RuntimeError::new(
            "VM: callback helper action lost its callback role",
        )),
        NativeDriverAction::FinishCallbackHelper { frame, response } => {
            let CallbackHelperActivationFrame {
                request,
                parent_role,
                ..
            } = frame;
            let CallbackHelperRequest { kind, deferred, .. } = request;
            let kind = match kind {
                CallbackHelperKind::DebugHook {
                    continuation,
                    trace_barrier,
                    ..
                } => {
                    state.debug_hook_active = false;
                    end_trace_callback_barrier(state, trace_barrier);
                    return match response {
                        ResumeResponse::Success(_) => {
                            resume_debug_hook_continuation(state, gc, *continuation, parent_role)
                        }
                        ResumeResponse::Error(error) => Err(RuntimeError::with_value(gc, error)),
                    };
                }
                CallbackHelperKind::VmMetamethod {
                    continuation,
                    trace_barrier,
                    ..
                } => {
                    return match response {
                        ResumeResponse::Success(values) => {
                            let completion =
                                finish_vm_metamethod_callback(state, gc, continuation, values);
                            end_trace_callback_barrier(state, trace_barrier);
                            let completion = completion?;
                            resume_after_callback_continuation(state, gc, parent_role, completion)
                        }
                        ResumeResponse::Error(error) => {
                            end_trace_callback_barrier(state, trace_barrier);
                            Err(RuntimeError::with_value(gc, error))
                        }
                    };
                }
                kind => kind,
            };
            let deferred = deferred.ok_or_else(|| {
                RuntimeError::new("callback helper activation lost its deferred call")
            })?;
            let completion = match (kind, response) {
                (CallbackHelperKind::ToString, ResumeResponse::Success(values)) => {
                    finish_deferred_runtime_result(
                        state,
                        gc,
                        &deferred,
                        false,
                        Ok(vec![values.into_iter().next().unwrap_or(Value::Nil)]),
                    )
                }
                (CallbackHelperKind::ToString, ResumeResponse::Error(error)) => {
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::LoadReader {
                        terminal_error: Some(message),
                        ..
                    },
                    _,
                ) => finish_deferred_load_error_message(state, gc, &deferred, &message),
                (
                    CallbackHelperKind::LoadReader {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Error(error),
                ) => finish_deferred_native_values(state, gc, &deferred, vec![Value::Nil, error]),
                (
                    CallbackHelperKind::LoadReader {
                        chunk_name,
                        source,
                        terminal_error: None,
                    },
                    ResumeResponse::Success(values),
                ) => match values.into_iter().next().unwrap_or(Value::Nil) {
                    Value::Nil | Value::String(_) => {
                        finish_deferred_load_source(state, gc, &deferred, &source, &chunk_name)
                    }
                    _ => finish_deferred_load_error_message(
                        state,
                        gc,
                        &deferred,
                        "reader function must return a string",
                    ),
                },
                (
                    CallbackHelperKind::TableSort {
                        terminal_error: Some(message),
                        ..
                    },
                    _,
                ) => {
                    let error = runtime_error_value(state, gc, &RuntimeError::new(message));
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::TableSort {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Error(error),
                ) => finish_deferred_runtime_result(state, gc, &deferred, false, Err(error)),
                (
                    CallbackHelperKind::TableSort {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Success(_),
                ) => finish_deferred_runtime_result(state, gc, &deferred, false, Ok(Vec::new())),
                (CallbackHelperKind::DoFile, ResumeResponse::Success(values)) => {
                    finish_deferred_runtime_result(state, gc, &deferred, false, Ok(values))
                }
                (CallbackHelperKind::DoFile, ResumeResponse::Error(error)) => {
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (CallbackHelperKind::TableForeach { .. }, ResumeResponse::Success(values)) => {
                    finish_deferred_runtime_result(
                        state,
                        gc,
                        &deferred,
                        false,
                        Ok(vec![values.into_iter().next().unwrap_or(Value::Nil)]),
                    )
                }
                (CallbackHelperKind::TableForeach { .. }, ResumeResponse::Error(error)) => {
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::PackageRequire { loaded, module_key },
                    ResumeResponse::Success(values),
                ) => {
                    let values = finish_package_require_callback(gc, loaded, &module_key, values)?;
                    finish_deferred_runtime_result(state, gc, &deferred, false, Ok(values))
                }
                (CallbackHelperKind::PackageRequire { .. }, ResumeResponse::Error(error)) => {
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::PackageModuleOptions {
                        terminal_error: Some(message),
                        ..
                    },
                    _,
                ) => {
                    let error = runtime_error_value(state, gc, &RuntimeError::new(message));
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::PackageModuleOptions {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Error(error),
                ) => finish_deferred_runtime_result(state, gc, &deferred, false, Err(error)),
                (
                    CallbackHelperKind::PackageModuleOptions {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Success(_),
                ) => finish_deferred_runtime_result(state, gc, &deferred, false, Ok(Vec::new())),
                (
                    CallbackHelperKind::Gsub {
                        terminal_error: Some(message),
                        ..
                    },
                    _,
                ) => {
                    let error = runtime_error_value(state, gc, &RuntimeError::new(message));
                    finish_deferred_runtime_result(state, gc, &deferred, false, Err(error))
                }
                (
                    CallbackHelperKind::Gsub {
                        terminal_error: None,
                        ..
                    },
                    ResumeResponse::Error(error),
                ) => finish_deferred_runtime_result(state, gc, &deferred, false, Err(error)),
                (
                    CallbackHelperKind::Gsub {
                        state: gsub,
                        terminal_error: None,
                    },
                    ResumeResponse::Success(_),
                ) => finish_deferred_gsub_result(state, gc, &deferred, &gsub.output, gsub.next),
                (CallbackHelperKind::DebugHook { .. }, _) => {
                    unreachable!("debug hook handled before deferred callback helpers")
                }
                (CallbackHelperKind::VmMetamethod { .. }, _) => {
                    unreachable!("VM metamethod handled before deferred callback helpers")
                }
            }?;
            resume_after_deferred_completion(state, gc, &deferred, parent_role, completion)
        }
        NativeDriverAction::DeliverUpvalue { frame, resume } => {
            if state.state_handle() != Some(frame.request.requester) {
                return Err(RuntimeError::new(
                    "open Upvalue response reached a different requester state",
                ));
            }
            let response = frame.response.ok_or_else(|| {
                RuntimeError::new("open Upvalue transfer has no owner-state response")
            })?;
            match (frame.request.operation, response) {
                (
                    UpvalueAccessOperation::Read { destination },
                    UpvalueAccessResponse::Read(value),
                ) => {
                    let slot = state.stack.at_mut(destination).ok_or_else(|| {
                        RuntimeError::new(
                            "open Upvalue read destination is outside the requester stack",
                        )
                    })?;
                    *slot = value;
                }
                (UpvalueAccessOperation::Write { .. }, UpvalueAccessResponse::Written) => {}
                _ => {
                    return Err(RuntimeError::new(
                        "open Upvalue response did not match its requested operation",
                    ));
                }
            }
            resume_after_suspension_for_role(state, gc, resume)
        }
        NativeDriverAction::DeliverDebugUpvalue { frame, resume } => {
            if state.state_handle() != Some(frame.request.requester) {
                return Err(RuntimeError::new(
                    "debug Upvalue response reached a different requester state",
                ));
            }
            let deferred = frame.request.deferred.as_ref().ok_or_else(|| {
                RuntimeError::new("debug Upvalue activation lost its deferred call")
            })?;
            let response = match frame.response {
                Some(DebugUpvalueResponse::Success(values)) => Ok(values),
                Some(DebugUpvalueResponse::ErrorMessage(message)) => {
                    Err(runtime_error_value(state, gc, &RuntimeError::new(message)))
                }
                None => {
                    return Err(RuntimeError::new(
                        "debug Upvalue transfer has no owner-state response",
                    ));
                }
            };
            let completion = finish_deferred_runtime_result(
                state,
                gc,
                deferred,
                frame.request.protected,
                response,
            )?;
            resume_after_deferred_completion(state, gc, deferred, resume, completion)
        }
        NativeDriverAction::DeliverCollection {
            request,
            response,
            resume,
        } => {
            let deferred = request.deferred.as_ref().ok_or_else(|| {
                RuntimeError::new("full collection activation lost its deferred call")
            })?;
            let response = match response {
                CollectionResponse::Success(values) => Ok(values),
                CollectionResponse::ErrorMessage(message) => {
                    Err(runtime_error_value(state, gc, &RuntimeError::new(message)))
                }
                CollectionResponse::ErrorValue(error) => Err(error),
            };
            let completion =
                finish_deferred_runtime_result(state, gc, deferred, request.protected, response)?;
            resume_after_deferred_completion(state, gc, deferred, resume, completion)
        }
        NativeDriverAction::ResumeAutomaticGc { resume } => {
            state.status = ThreadStatus::Yield;
            resume_after_suspension_for_role(state, gc, resume)
        }
        NativeDriverAction::StartFinalizer {
            callback,
            userdata,
            stop_ci,
        } => start_finalizer_callback(state, gc, callback, userdata, stop_ci),
        NativeDriverAction::FailFinalizer {
            message,
            stop_ci,
            saved_top,
        } => {
            abort_finalizer_callback(state, gc, stop_ci, saved_top)?;
            Err(RuntimeError::new(message))
        }
    }
}

fn execute_upvalue_owner_access(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    request: &UpvalueAccessRequest,
) -> Result<UpvalueAccessResponse, RuntimeError> {
    if state.state_handle() != Some(request.owner) {
        return Err(RuntimeError::new(
            "open Upvalue owner-state handle did not match the resolved state",
        ));
    }
    if !state.open_upvalues.contains(&request.upvalue) {
        return Err(RuntimeError::new(
            "open Upvalue owner state no longer contains the requested Upvalue",
        ));
    }
    let location = gc
        .with_ref(request.upvalue, Upvalue::open_location)
        .map_err(|error| RuntimeError::new(format!("invalid open Upvalue request: {error}")))?;
    if location != Some((request.owner, request.stack_index)) {
        return Err(RuntimeError::new(
            "open Upvalue location changed before owner-state access",
        ));
    }

    match &request.operation {
        UpvalueAccessOperation::Read { .. } => {
            let value = state
                .stack
                .at(request.stack_index)
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::new("open Upvalue owner stack index is out of range")
                })?;
            Ok(UpvalueAccessResponse::Read(value))
        }
        UpvalueAccessOperation::Write { value } => {
            let slot = state.stack.at_mut(request.stack_index).ok_or_else(|| {
                RuntimeError::new("open Upvalue owner stack index is out of range")
            })?;
            *slot = value.clone();
            Ok(UpvalueAccessResponse::Written)
        }
    }
}

fn execute_debug_upvalue_owner_access(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    request: &DebugUpvalueRequest,
) -> Result<Vec<Value>, RuntimeError> {
    if state.state_handle() != Some(request.owner) {
        return Err(RuntimeError::new(
            "debug Upvalue owner-state handle did not match the resolved state",
        ));
    }
    if !state.open_upvalues.contains(&request.upvalue) {
        return Err(RuntimeError::new(
            "debug Upvalue owner state no longer contains the requested Upvalue",
        ));
    }
    let location = gc
        .with_ref(request.upvalue, Upvalue::open_location)
        .map_err(|error| RuntimeError::new(format!("invalid debug Upvalue request: {error}")))?;
    if location != Some((request.owner, request.stack_index)) {
        return Err(RuntimeError::new(
            "debug Upvalue location changed before owner-state access",
        ));
    }

    match &request.operation {
        DebugUpvalueOperation::Read => {
            let value = state
                .stack
                .at(request.stack_index)
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::new("debug Upvalue owner stack index is out of range")
                })?;
            Ok(vec![request.name.clone(), value])
        }
        DebugUpvalueOperation::Write { value } => {
            let slot = state.stack.at_mut(request.stack_index).ok_or_else(|| {
                RuntimeError::new("debug Upvalue owner stack index is out of range")
            })?;
            *slot = value.clone();
            Ok(vec![request.name.clone()])
        }
    }
}

fn resume_after_deferred_for_role(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    role: NativeDriverRole,
) -> Result<VmExit, RuntimeError> {
    match role {
        NativeDriverRole::Program => resume_after_deferred_native_call(state, gc, deferred),
        NativeDriverRole::Finalizer { stop_ci } => resume_finalizer_callback(state, gc, stop_ci),
        NativeDriverRole::ProtectedCall { stop_ci, .. }
        | NativeDriverRole::CallbackHelper { stop_ci, .. } => {
            resume_runtime_callback(state, gc, stop_ci)
        }
    }
}

fn resume_after_deferred_completion(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    role: NativeDriverRole,
    completion: VmExit,
) -> Result<VmExit, RuntimeError> {
    match completion {
        VmExit::Complete(ExecResult::Returned) => {
            resume_after_deferred_for_role(state, gc, deferred, role)
        }
        completion => Ok(completion),
    }
}

fn resume_debug_hook_continuation(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    continuation: DebugHookContinuation,
    parent_role: NativeDriverRole,
) -> Result<VmExit, RuntimeError> {
    let completion = match continuation {
        DebugHookContinuation::LuaCall { proto } => {
            state.status = ThreadStatus::Ok;
            resume_lua_call_after_debug_hook(state, proto, gc)?
        }
        DebugHookContinuation::ResumeAt { proto, pc } => {
            resume_lua_instruction_after_debug_hook(state, proto, pc, gc)?
        }
        DebugHookContinuation::InstructionAfterCount { proto, pc } => {
            if let Some(request_id) = dispatch_debug_line_hook(state, gc, proto, pc)? {
                return Ok(VmExit::NativeRequest(request_id));
            }
            resume_lua_instruction_after_debug_hook(state, proto, pc, gc)?
        }
        DebugHookContinuation::LuaReturn {
            call_info,
            results,
            remaining_tail_returns,
        } => {
            if remaining_tail_returns > 0
                && let Some(request_id) = publish_debug_hook_event(
                    state,
                    gc,
                    "tail return",
                    None,
                    DebugHookContinuation::LuaReturn {
                        call_info: call_info.clone(),
                        results: results.clone(),
                        remaining_tail_returns: remaining_tail_returns - 1,
                    },
                )?
            {
                return Ok(VmExit::NativeRequest(request_id));
            }
            finish_lua_return_after_debug_hook(state, &call_info, results)?
        }
        DebugHookContinuation::NativeCall { frame } => {
            resume_native_call_after_debug_hook(state, gc, frame)?
        }
        DebugHookContinuation::NativeReturn {
            frame,
            result_count,
        } => finish_native_call_after_debug_hook(state, gc, frame, result_count)?,
    };

    resume_after_callback_continuation(state, gc, parent_role, completion)
}

fn resume_after_callback_continuation(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    parent_role: NativeDriverRole,
    completion: VmExit,
) -> Result<VmExit, RuntimeError> {
    match completion {
        VmExit::Complete(ExecResult::Returned) => {
            if matches!(parent_role, NativeDriverRole::Program)
                && state.current_ci == 0
                && state.current_call_info().proto.is_none()
            {
                return Ok(VmExit::Complete(ExecResult::Returned));
            }
            state.status = ThreadStatus::Yield;
            resume_after_suspension_for_role(state, gc, parent_role)
        }
        completion => Ok(completion),
    }
}

fn resume_after_suspension_for_role(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    role: NativeDriverRole,
) -> Result<VmExit, RuntimeError> {
    match role {
        NativeDriverRole::Program => resume_after_upvalue_access(state, gc),
        NativeDriverRole::Finalizer { stop_ci } => resume_finalizer_callback(state, gc, stop_ci),
        NativeDriverRole::ProtectedCall { stop_ci, .. }
        | NativeDriverRole::CallbackHelper { stop_ci, .. } => {
            resume_runtime_callback(state, gc, stop_ci)
        }
    }
}

fn resolve_debug_upvalue_request(
    arena: NonNull<StateArena>,
    gc: &mut GarbageCollector,
    string_pool: &mut StringPool,
    runtime_id: RuntimeId,
    request: &DebugUpvalueRequest,
) -> DebugUpvalueResponse {
    match StateArena::with_turn_state_mut(arena, request.owner, |owner_state| {
        with_vm_context_gc(owner_state, gc, string_pool, |owner_state, gc| {
            execute_debug_upvalue_owner_access(owner_state, gc, request)
        })
    }) {
        Ok(Ok(values)) => DebugUpvalueResponse::Success(values),
        Ok(Err(error)) => DebugUpvalueResponse::ErrorMessage(error.to_string()),
        Err(source) => DebugUpvalueResponse::ErrorMessage(
            RuntimeAccessError::StateArena { runtime_id, source }.to_string(),
        ),
    }
}

fn resume_after_upvalue_access(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    if state.current_ci == 0 {
        let proto = state
            .current_call_info()
            .proto
            .ok_or_else(|| RuntimeError::new("open Upvalue requester lost its root Proto"))?;
        return execute_proto(state, proto, gc);
    }

    let result = resume_lua_thread(state, gc)?;
    if matches!(result, VmExit::Complete(ExecResult::Returned))
        && state.current_ci == 0
        && let Some(root_proto) = state.current_call_info().proto
    {
        state.status = ThreadStatus::Yield;
        return execute_proto(state, root_proto, gc);
    }
    Ok(result)
}

fn classify_native_turn(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    result: Result<VmExit, RuntimeError>,
    role: NativeDriverRole,
) -> Result<NativeTurnEvent, RuntimeError> {
    match result {
        Ok(VmExit::NativeRequest(id)) => {
            let request = state.take_native_request(id).ok_or_else(|| {
                RuntimeError::new("VM exited for a missing or unsealed native request")
            })?;
            Ok(NativeTurnEvent::Request {
                request: Box::new(request),
                caller_thread: state.current_thread,
            })
        }
        Ok(VmExit::UpvalueAccess(request)) => Ok(NativeTurnEvent::UpvalueAccess(request)),
        Ok(VmExit::AutomaticGc) => Ok(NativeTurnEvent::AutomaticGc {
            stop_ci: state.current_ci,
            saved_top: state.top,
        }),
        Ok(VmExit::Complete(ExecResult::Returned))
            if matches!(role, NativeDriverRole::CallbackHelper { .. }) =>
        {
            let NativeDriverRole::CallbackHelper {
                stop_ci,
                call_pos,
                saved_top,
            } = role
            else {
                unreachable!("guarded callback helper role")
            };
            let values = finish_runtime_callback_turn(state, stop_ci, call_pos, saved_top)?;
            Ok(NativeTurnEvent::CallbackHelper(ResumeResponse::Success(
                values,
            )))
        }
        Ok(VmExit::Complete(ExecResult::Yielded))
            if matches!(role, NativeDriverRole::CallbackHelper { .. }) =>
        {
            let NativeDriverRole::CallbackHelper {
                stop_ci, saved_top, ..
            } = role
            else {
                unreachable!("guarded callback helper role")
            };
            abort_runtime_callback_turn(state, gc, stop_ci, saved_top)?;
            let error = runtime_error_value(
                state,
                gc,
                &RuntimeError::new("cannot yield across a callback helper"),
            );
            Ok(NativeTurnEvent::CallbackHelper(ResumeResponse::Error(
                error,
            )))
        }
        Ok(VmExit::Complete(ExecResult::Returned))
            if matches!(role, NativeDriverRole::ProtectedCall { .. }) =>
        {
            let NativeDriverRole::ProtectedCall {
                stop_ci,
                call_pos,
                saved_top,
            } = role
            else {
                unreachable!("guarded protected helper role")
            };
            let values = finish_protected_callback_turn(state, stop_ci, call_pos, saved_top)?;
            Ok(NativeTurnEvent::ProtectedCall(ResumeResponse::Success(
                values,
            )))
        }
        Ok(VmExit::Complete(ExecResult::Yielded))
            if matches!(role, NativeDriverRole::ProtectedCall { .. }) =>
        {
            let NativeDriverRole::ProtectedCall {
                stop_ci, saved_top, ..
            } = role
            else {
                unreachable!("guarded protected helper role")
            };
            abort_runtime_callback_turn(state, gc, stop_ci, saved_top)?;
            let error =
                runtime_error_value(state, gc, &RuntimeError::new("cannot yield across pcall"));
            Ok(NativeTurnEvent::ProtectedCall(ResumeResponse::Error(error)))
        }
        Ok(VmExit::Complete(ExecResult::Returned))
            if matches!(role, NativeDriverRole::Finalizer { .. }) =>
        {
            Ok(NativeTurnEvent::Finalizer(Ok(())))
        }
        Ok(VmExit::Complete(ExecResult::Yielded))
            if matches!(role, NativeDriverRole::Finalizer { .. }) =>
        {
            Ok(NativeTurnEvent::Finalizer(Err(RuntimeError::new(
                "attempt to yield from a __gc metamethod",
            ))))
        }
        Ok(VmExit::Complete(result)) => {
            let Some(thread) = state.current_thread else {
                return Ok(NativeTurnEvent::Main(Ok(result)));
            };
            release_yield_permission(state)?;
            let response = match result {
                ExecResult::Yielded => {
                    gc.with_mut(thread, |thread| {
                        thread.set_status(CoroutineStatus::Suspended);
                        thread.set_saved_nexeccalls(state.nccalls);
                    })
                    .map_err(|error| {
                        RuntimeError::new(format!("invalid coroutine Thread: {error}"))
                    })?;
                    state.last_error = None;
                    ResumeResponse::Success(state.yielded_values.clone())
                }
                ExecResult::Returned => {
                    gc.with_mut(thread, |thread| {
                        thread.set_status(CoroutineStatus::Dead);
                    })
                    .map_err(|error| {
                        RuntimeError::new(format!("invalid coroutine Thread: {error}"))
                    })?;
                    state.last_error = None;
                    ResumeResponse::Success(state_stack_values(state))
                }
            };
            Ok(NativeTurnEvent::Coroutine { thread, response })
        }
        Err(error) => {
            if let NativeDriverRole::CallbackHelper {
                stop_ci, saved_top, ..
            } = role
            {
                let error = runtime_error_value(state, gc, &error);
                abort_runtime_callback_turn(state, gc, stop_ci, saved_top)?;
                return Ok(NativeTurnEvent::CallbackHelper(ResumeResponse::Error(
                    error,
                )));
            }
            if let NativeDriverRole::ProtectedCall {
                stop_ci, saved_top, ..
            } = role
            {
                let error = runtime_error_value(state, gc, &error.located());
                abort_runtime_callback_turn(state, gc, stop_ci, saved_top)?;
                return Ok(NativeTurnEvent::ProtectedCall(ResumeResponse::Error(error)));
            }
            if matches!(role, NativeDriverRole::Finalizer { .. }) {
                return Ok(NativeTurnEvent::Finalizer(Err(error)));
            }
            let Some(thread) = state.current_thread else {
                unwind_top_level_lua_error(state, gc)?;
                discard_all_instruction_diffs(state);
                return Ok(NativeTurnEvent::Main(Err(error.located())));
            };
            discard_all_instruction_diffs(state);
            release_yield_permission(state)?;
            gc.with_mut(thread, |thread| {
                thread.set_status(CoroutineStatus::Dead);
            })
            .map_err(|validation| {
                RuntimeError::new(format!("invalid coroutine Thread: {validation}"))
            })?;
            let error = runtime_error_value(state, gc, &error);
            Ok(NativeTurnEvent::Coroutine {
                thread,
                response: ResumeResponse::Error(error),
            })
        }
    }
}

fn prepare_native_activation(
    gc: &mut GarbageCollector,
    caller: StateHandle,
    caller_thread: Option<GcRef<Thread>>,
    request: ResumeRequest,
) -> Result<NativeActivationFrame, RuntimeExecutionError> {
    if request.deferred.is_none() {
        return Err(RuntimeExecutionError::Protocol(
            "native request reached Runtime without deferred call metadata".to_string(),
        ));
    }
    let (previous_target_status, previous_target_caller) = gc
        .with_ref(request.thread, |thread| (thread.status(), thread.caller()))
        .map_err(|error| {
            RuntimeExecutionError::Protocol(format!("invalid target Thread: {error}"))
        })?;
    if matches!(
        previous_target_status,
        CoroutineStatus::Running | CoroutineStatus::Dead
    ) {
        return Err(RuntimeExecutionError::Protocol(format!(
            "native request targeted a {previous_target_status} coroutine"
        )));
    }

    let previous_caller_status = match caller_thread {
        Some(thread) => Some(gc.with_ref(thread, Thread::status).map_err(|error| {
            RuntimeExecutionError::Protocol(format!("invalid caller Thread: {error}"))
        })?),
        None => None,
    };
    if let Some(thread) = caller_thread {
        gc.with_mut(thread, |thread| {
            thread.set_status(CoroutineStatus::Normal);
        })
        .map_err(|error| {
            RuntimeExecutionError::Protocol(format!("invalid caller Thread: {error}"))
        })?;
    }
    gc.with_mut(request.thread, |thread| {
        thread.set_caller(caller_thread);
        thread.set_status(CoroutineStatus::Running);
    })
    .map_err(|error| RuntimeExecutionError::Protocol(format!("invalid target Thread: {error}")))?;

    Ok(NativeActivationFrame {
        caller,
        caller_thread,
        previous_caller_status,
        previous_target_status,
        previous_target_caller,
        request,
        pending_response: None,
        replay_dead_ancestor: false,
    })
}

fn restore_native_activation_links(
    gc: &mut GarbageCollector,
    frame: &NativeActivationFrame,
) -> Result<(), RuntimeExecutionError> {
    gc.with_mut(frame.request.thread, |thread| {
        thread.set_caller(frame.previous_target_caller);
    })
    .map_err(|error| RuntimeExecutionError::Protocol(format!("invalid target Thread: {error}")))?;
    if let (Some(caller), Some(previous_status)) =
        (frame.caller_thread, frame.previous_caller_status)
    {
        gc.with_mut(caller, |thread| {
            if thread.status() != CoroutineStatus::Dead {
                thread.set_status(previous_status);
            }
        })
        .map_err(|error| {
            RuntimeExecutionError::Protocol(format!("invalid caller Thread: {error}"))
        })?;
    }
    Ok(())
}

fn install_initial_resume_args(state: &mut LuaState, args: &[Value]) {
    ensure_state_stack_slot(state, args.len());
    for (index, value) in args.iter().enumerate() {
        if let Some(destination) = state.stack.at_mut(1 + index) {
            *destination = value.clone();
        }
    }
    state.top = 1 + args.len();
}

fn install_resume_args(state: &mut LuaState, args: &[Value]) {
    let base = state.yield_result_base.take().unwrap_or(state.top);
    let wanted = state.yield_wanted_results.take().unwrap_or(args.len());
    if wanted > 0 {
        ensure_state_stack_slot(state, base + wanted - 1);
    }
    for index in 0..wanted {
        let value = args.get(index).cloned().unwrap_or(Value::Nil);
        if let Some(destination) = state.stack.at_mut(base + index) {
            *destination = value;
        }
    }
    state.top = base + wanted;
    state.yielded_values.clear();
}

fn ensure_state_stack_slot(state: &mut LuaState, index: usize) {
    if state.stack.size() <= index {
        state.stack.set_top(index + 1);
    }
}

fn state_stack_values(state: &LuaState) -> Vec<Value> {
    (0..state.top)
        .map(|index| state.stack.at(index).cloned().unwrap_or(Value::Nil))
        .collect()
}

struct TopLevelStateResetGuard<'state> {
    state: &'state mut LuaState,
}

impl Drop for TopLevelStateResetGuard<'_> {
    fn drop(&mut self) {
        self.state.unwind_call_info_to(0);
        self.state.current_call_info_mut().reset();
        self.state.top = 0;
    }
}

fn runtime_error_value(
    state: &mut LuaState,
    gc: &mut GarbageCollector,
    error: &RuntimeError,
) -> Value {
    if let Some(value) = error.error_value() {
        state.last_error = Some(value.clone());
        return value;
    }

    gc.with_publication(|transaction| {
        let pool = state
            .active_string_pool_ptr()
            .ok_or(GcRefValidationError::StringPoolUnavailable)?;
        // SAFETY: this state turn owns the dynamically scoped Heap services.
        let message = transaction.intern_bytes(unsafe { &mut *pool }, error.message.as_bytes())?;
        // SAFETY: last_error is a Runtime-traced LuaState owner and receives
        // the string before its temporary publication root is released.
        unsafe {
            transaction.publish_string_value(message, |value| {
                state.last_error = Some(value.clone());
                value
            })
        }
    })
    .expect("Runtime errors require the active canonical StringPool")
}

fn release_yield_permission(state: &mut LuaState) -> Result<(), RuntimeError> {
    state.allow_yield = state
        .allow_yield
        .checked_sub(1)
        .ok_or_else(|| RuntimeError::new("coroutine yield permission underflow"))?;
    Ok(())
}

struct NativeActivationSessionGuard<'session> {
    activations: &'session mut NativeActivationStack,
    gc: &'session mut GarbageCollector,
    arena: NonNull<StateArena>,
}

impl Drop for NativeActivationSessionGuard<'_> {
    fn drop(&mut self) {
        let activations = &mut *self.activations;
        let gc = &mut *self.gc;
        // A fault can leave one or more nested protected-call barriers installed
        // after the last state turn has ended. Restore them innermost-first so a
        // nested frame saved under an outer barrier cannot make its requester
        // spuriously yieldable while the outer frame is still active.
        while let Some(frame) = activations.protected_calls.pop() {
            let _ = StateArena::with_turn_state_mut(self.arena, frame.requester, |state| {
                state.allow_yield = frame.previous_allow_yield;
            });
        }
        while let Some(frame) = activations.frames.pop() {
            if let Some(caller) = frame.caller_thread
                && let Some(previous_status) = frame.previous_caller_status
            {
                let _ = gc.with_mut(caller, |thread| {
                    thread.set_status(previous_status);
                });
            }
            let _ = gc.with_mut(frame.request.thread, |thread| {
                thread.set_status(frame.previous_target_status);
                thread.set_caller(frame.previous_target_caller);
            });
        }
        activations.upvalue_transfers.clear();
        activations.debug_upvalue_transfers.clear();
        activations.callback_helpers.clear();
        if activations
            .gc_frames
            .iter()
            .any(|frame| frame.draining_finalizers)
        {
            gc.end_finalizer_drain();
        }
        activations.gc_frames.clear();
        gc.end_automatic_checkpoint_driver();
    }
}

struct ActiveExecutionGuard<'runtime> {
    active_executions: &'runtime mut usize,
}

impl Drop for ActiveExecutionGuard<'_> {
    fn drop(&mut self) {
        *self.active_executions = self
            .active_executions
            .checked_sub(1)
            .expect("active Runtime session owns one execution count");
    }
}

/// Scoped mutable access to the three execution components.
///
/// [`Self::split_mut`] exposes explicit component borrows with no dynamic Heap
/// context. [`Self::with_context`] and [`Self::with_context_gc`] install native
/// services only inside a callback which exclusively borrows this whole guard.
#[must_use = "dropping the guard ends the active execution borrow"]
pub struct RuntimePartsMut<'runtime> {
    state: &'runtime mut LuaState,
    gc: &'runtime mut GarbageCollector,
    string_pool: &'runtime mut StringPool,
    state_arena: NonNull<StateArena>,
    state_handle: StateHandle,
    active_executions: &'runtime mut usize,
}

impl RuntimePartsMut<'_> {
    /// Borrow all execution components at once, without dynamic Heap access.
    ///
    /// Use the returned collector for managed-object reads. State-side dynamic
    /// access fails with `CollectorUnavailable`, so a callback cannot hold a
    /// managed borrow while separately using this mutable collector to sweep.
    pub fn split_mut(&mut self) -> (&mut LuaState, &mut GarbageCollector, &mut StringPool) {
        (self.state, self.gc, self.string_pool)
    }

    /// Run a native operation with dynamic Heap access and no explicit service
    /// references available to the callback.
    ///
    /// The callback cannot acquire an independent collector borrow:
    ///
    /// ```compile_fail
    /// let mut runtime = lua_vm::Runtime::new();
    /// let mut parts = runtime.parts_mut().unwrap();
    /// parts.with_context(|state| {
    ///     let (_, gc, strings) = parts.split_mut();
    ///     gc.collect(strings);
    ///     state.push_nil();
    /// });
    /// ```
    pub fn with_context<R>(&mut self, execute: impl FnOnce(&mut LuaState) -> R) -> R {
        with_vm_context(self.state, self.gc, self.string_pool, execute)
    }

    /// Run initialization or VM code with an explicit collector and a dynamic
    /// StringPool. Dynamic collector recovery is disabled during this callback.
    ///
    /// The StringPool is deliberately not passed to the callback: interning
    /// helpers borrow it through the state without aliasing an explicit borrow.
    pub fn with_context_gc<R>(
        &mut self,
        execute: impl FnOnce(&mut LuaState, &mut GarbageCollector) -> R,
    ) -> R {
        with_vm_context_gc(self.state, self.gc, self.string_pool, execute)
    }

    /// Return the main state through a shared borrow.
    pub fn state(&self) -> &LuaState {
        self.state
    }

    /// Return the active count represented by this guard.
    pub fn active_execution_count(&self) -> usize {
        *self.active_executions
    }
}

impl Drop for RuntimePartsMut<'_> {
    fn drop(&mut self) {
        let mut arena = self.state_arena;
        // SAFETY: RuntimePartsMut is tied to the exclusive Runtime borrow, so
        // its pinned heap and StateArena outlive this guard.
        let arena = unsafe { arena.as_mut() };
        let updated =
            arena.update_state_allocation(self.state_handle, NonNull::from(&mut *self.state));
        debug_assert!(updated);
        let released = arena.release(self.state_handle);
        debug_assert!(released);
        debug_assert!(*self.active_executions > 0);
        *self.active_executions = self.active_executions.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use lua_core::userdata::Userdata;

    use super::*;
    use crate::native::NativeRequestId;

    static RESOURCE_DRAIN_CALLS: AtomicUsize = AtomicUsize::new(0);
    static PREFLIGHT_RESOURCE_DRAIN_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn activation_cleanup_restores_nested_protected_yield_barriers_in_stack_order() {
        let issuer =
            StateHandleIssuer::try_new().expect("test Runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let requester = arena.insert_owned(Box::new(LuaState::new()));
        StateArena::with_turn_state_mut(NonNull::from(&mut arena), requester, |state| {
            state.allow_yield = 0;
        })
        .expect("requester state is available");

        let role = NativeDriverRole::ProtectedCall {
            stop_ci: 0,
            call_pos: 0,
            saved_top: 0,
        };
        let frame = |id, previous_allow_yield| ProtectedCallActivationFrame {
            requester,
            request: ProtectedCallRequest {
                id: NativeRequestId::new(id),
                kind: ProtectedCallKind::PCall,
                function: Value::Nil,
                args: Vec::new(),
                handler: None,
                deferred: None,
            },
            phase: ProtectedCallPhase::Target,
            role,
            parent_role: NativeDriverRole::Program,
            previous_allow_yield,
            pending_response: None,
            sequence: id,
        };
        let mut activations = NativeActivationStack::default();
        activations.protected_calls.push(frame(1, 1));
        activations.protected_calls.push(frame(2, 0));
        let mut gc = GarbageCollector::new();
        gc.begin_automatic_checkpoint_driver();

        {
            let _cleanup = NativeActivationSessionGuard {
                activations: &mut activations,
                gc: &mut gc,
                arena: NonNull::from(&mut arena),
            };
        }

        assert!(activations.protected_calls.is_empty());
        StateArena::with_turn_state_mut(NonNull::from(&mut arena), requester, |state| {
            assert_eq!(state.allow_yield, 1);
        })
        .expect("cleanup leaves requester state available");
    }

    fn count_resource_drain(
        _state: &mut LuaState,
        gc: &mut GarbageCollector,
        object_id: ObjectId,
    ) -> RuntimeResourceDrainOutcome {
        assert!(gc.registered_ref_by_id::<Userdata>(object_id).is_some());
        RESOURCE_DRAIN_CALLS.fetch_add(1, Ordering::SeqCst);
        RuntimeResourceDrainOutcome::Drained
    }

    fn panic_resource_drain(
        _state: &mut LuaState,
        _gc: &mut GarbageCollector,
        _object_id: ObjectId,
    ) -> RuntimeResourceDrainOutcome {
        panic!("injected Runtime resource drain panic");
    }

    fn drain_resource_without_counter(
        _state: &mut LuaState,
        gc: &mut GarbageCollector,
        object_id: ObjectId,
    ) -> RuntimeResourceDrainOutcome {
        assert!(gc.registered_ref_by_id::<Userdata>(object_id).is_some());
        RuntimeResourceDrainOutcome::Drained
    }

    fn count_preflight_resource_drain(
        state: &mut LuaState,
        gc: &mut GarbageCollector,
        object_id: ObjectId,
    ) -> RuntimeResourceDrainOutcome {
        PREFLIGHT_RESOURCE_DRAIN_CALLS.fetch_add(1, Ordering::SeqCst);
        drain_resource_without_counter(state, gc, object_id)
    }

    fn move_runtime(runtime: Runtime) -> Runtime {
        runtime
    }

    #[test]
    fn try_new_issues_distinct_nonzero_runtime_namespaces() {
        let first = Runtime::try_new().expect("first runtime namespace is available");
        let second = Runtime::try_new().expect("second runtime namespace is available");

        assert_ne!(first.id(), second.id());
        assert_ne!(first.id().get(), 0);
        assert_ne!(second.id().get(), 0);
        assert_ne!(first.id().get(), u64::MAX);
        assert_ne!(second.id().get(), u64::MAX);
    }

    #[test]
    fn runtime_owns_and_traces_fixed_strings_until_heap_shutdown() {
        let mut runtime = Runtime::new();
        let expected = METAMETHOD_NAMES.len() + FIXED_RUNTIME_STRING_BYTES.len();
        assert_eq!(runtime.fixed_strings.len(), expected);
        let collector = runtime.heap.as_ref().get_ref().heap.collector();
        for string in &runtime.fixed_strings {
            assert!(
                collector
                    .with_ref(*string, GcString::is_fixed)
                    .expect("fixed Runtime string remains registered")
            );
        }

        let mark = runtime
            .trace_roots_mark_only()
            .expect("fixed Runtime strings participate in canonical tracing");
        assert_eq!(
            mark.root_edge_count(root_trace::RuntimeRootKind::FixedStrings),
            expected
        );

        let close = runtime.close().expect("Runtime should close");
        assert_eq!(close.destroyed_fixed_objects, expected);
        assert_eq!(close.remaining_objects, 0);
        assert_eq!(close.remaining_interned_strings, 0);
    }

    #[test]
    fn heap_services_are_scoped_and_remain_stable_when_runtime_moves() {
        let mut runtime = Runtime::new();
        let global_root = runtime.global_root().expect("global root is owned");
        let main_handle = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let main_state_address = runtime
            .main_state()
            .map(std::ptr::from_ref)
            .expect("running runtime has a main state");
        let arena = runtime.heap.as_ref().get_ref().state_arena();
        assert_eq!(
            arena
                .validate_main(main_handle)
                .expect("main handle resolves through the arena")
                .as_ptr(),
            main_state_address.cast_mut()
        );
        assert!(arena.slots[main_handle.slot()].is_main);
        assert_eq!(arena.live_coroutine_state_count(), 0);
        assert_eq!(
            arena
                .slots
                .iter()
                .filter(|slot| slot.state.is_some())
                .count(),
            1
        );
        let (gc_address, string_pool_address) = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            assert_eq!(parts.active_execution_count(), 1);
            let (state, gc, string_pool) = parts.split_mut();
            assert_eq!(state.global_table, Some(global_root));
            assert_eq!(state.state_handle(), Some(main_handle));
            assert!(state.state_arena.is_some());
            assert!(gc.is_root(global_root));
            assert!(state.active_gc_ptr().is_none());
            assert!(state.active_string_pool_ptr().is_none());
            (std::ptr::from_mut(gc), std::ptr::from_mut(string_pool))
        };
        assert!(
            runtime
                .main_state()
                .is_some_and(|state| state.active_gc_ptr().is_none())
        );

        let mut runtime = move_runtime(runtime);
        let mut parts = runtime.parts_mut().expect("parts survive Runtime move");
        let (state, gc, string_pool) = parts.split_mut();
        assert_eq!(std::ptr::from_ref(state), main_state_address);
        assert_eq!(std::ptr::from_mut(gc), gc_address);
        assert_eq!(std::ptr::from_mut(string_pool), string_pool_address);
        assert!(state.active_gc_ptr().is_none());
        assert!(state.active_string_pool_ptr().is_none());
        parts.with_context(|state| {
            assert!(state.active_gc_ptr().is_some());
            assert!(state.active_string_pool_ptr().is_some());
        });
    }

    #[test]
    fn pending_state_rolls_back_or_publishes_atomically() {
        let mut runtime = Runtime::new();
        let baseline_states = runtime.live_coroutine_state_count();

        let allocation_unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (_, gc, _) = parts.split_mut();
            gc.with_publication(|transaction| {
                let _thread = transaction.alloc(Thread::new());
                assert_eq!(transaction.active_temporary_root_count(), 1);
                panic!("injected failure after Thread allocation");
            });
        }));
        assert!(allocation_unwind.is_err());
        assert_eq!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .heap
                .collector()
                .temporary_root_count(),
            0
        );
        assert_eq!(runtime.live_coroutine_state_count(), baseline_states);

        let mut inserted_handle = None;
        let insertion_unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, _, _) = parts.split_mut();
            main.with_pending_coroutine_state(LuaState::new(), |pending, publisher| {
                inserted_handle = Some(pending.handle);
                assert_eq!(publisher.temporary_state_root_count(), 1);
                panic!("injected failure after pending-state insertion");
            })
            .expect("pending state can be inserted");
        }));
        assert!(insertion_unwind.is_err());
        let inserted_handle = inserted_handle.expect("pending handle was observed");
        assert_eq!(runtime.temporary_state_root_count(), 0);
        assert_eq!(runtime.live_coroutine_state_count(), baseline_states);
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .validate(inserted_handle),
            Err(StateResolveError::StaleGeneration { .. })
        ));

        let mut wrong_publisher = LuaState::new();
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, _, _) = parts.split_mut();
            let rejected = main
                .with_pending_coroutine_state(LuaState::new(), |pending, _| {
                    pending.publish_thread_to_stack(&mut wrong_publisher)
                })
                .expect("pending state can be inserted");
            assert!(matches!(
                rejected,
                Err(PendingStateError::PublisherMismatch)
            ));
        }
        assert_eq!(runtime.temporary_state_root_count(), 0);
        assert_eq!(runtime.live_coroutine_state_count(), baseline_states);

        let mut bound_handle = None;
        let binding_unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            gc.with_publication(|transaction| {
                let thread = transaction.alloc(Thread::new());
                main.with_pending_coroutine_state(LuaState::new(), |pending, publisher| {
                    pending
                        .bind_thread(transaction, thread)
                        .expect("both State↔Thread edges bind");
                    bound_handle = Some(pending.handle);
                    assert_eq!(publisher.temporary_state_root_count(), 1);
                    assert_eq!(transaction.active_temporary_root_count(), 0);
                    // SAFETY: this test is inside the branded PendingState
                    // callback, so its pinned arena and exact slot remain live.
                    let (_, state) = unsafe { pending.arena.as_ref() }
                        .validate_pending(pending.handle, pending.temporary_root_id)
                        .expect("pending slot remains exact");
                    // SAFETY: the pending root owns this unpublished state for
                    // the lexical callback.
                    let thread_ref = unsafe { state.as_ref() }
                        .current_thread
                        .expect("State→Thread edge is installed");
                    assert_eq!(Some(thread_ref.object_id()), pending.bound_thread);
                    let protected = transaction
                        .protect(thread_ref)
                        .expect("the State edge names a live Thread");
                    assert_eq!(
                        transaction
                            .with_ref(&protected, Thread::state_handle)
                            .expect("protected Thread remains valid"),
                        Some(pending.handle)
                    );
                    panic!("injected failure after State↔Thread binding");
                })
                .expect("pending state can be inserted");
            });
        }));
        assert!(binding_unwind.is_err());
        let bound_handle = bound_handle.expect("bound handle was observed");
        assert_eq!(runtime.temporary_state_root_count(), 0);
        assert_eq!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .heap
                .collector()
                .temporary_root_count(),
            0
        );
        assert_eq!(runtime.live_coroutine_state_count(), baseline_states);
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .validate(bound_handle),
            Err(StateResolveError::StaleGeneration { .. })
        ));

        let published_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            gc.with_publication(|transaction| {
                let thread = transaction.alloc(Thread::new());
                main.with_pending_coroutine_state(LuaState::new(), |pending, publisher| {
                    pending
                        .bind_thread(transaction, thread)
                        .expect("both State↔Thread edges bind");
                    pending.publish_thread_to_stack(publisher)
                })
                .expect("pending state can be inserted")
                .expect("bound Thread publishes to the stack")
            })
        };

        assert_eq!(runtime.temporary_state_root_count(), 0);
        assert_eq!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .heap
                .collector()
                .temporary_root_count(),
            0
        );
        assert_eq!(runtime.live_coroutine_state_count(), baseline_states + 1);
        assert_eq!(runtime.rejected_temporary_state_root_release_count(), 0);
        assert_eq!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .heap
                .collector()
                .rejected_temporary_root_release_count(),
            0
        );

        let mut parts = runtime.parts_mut().expect("parts are available");
        let (main, gc, _) = parts.split_mut();
        let thread = match main.stack.top_value().cloned() {
            Some(Value::Thread(thread)) => thread,
            other => panic!("published stack value was not a Thread: {other:?}"),
        };
        assert_eq!(
            gc.with_ref(thread, Thread::state_handle)
                .expect("published Thread remains registered"),
            Some(published_handle)
        );
        main.with_resolved_state_mut(published_handle, |state| {
            assert_eq!(state.current_thread, Some(thread));
        })
        .expect("published state resolves through its checked handle");
    }

    #[test]
    fn state_open_upvalues_are_unique_owned_and_sorted_by_descending_stack_index() {
        let mut runtime = Runtime::new();
        let (owner, low, middle, high, duplicate_high) = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            let owner = main
                .insert_coroutine_state(LuaState::new())
                .expect("owner state is inserted");
            let (low, middle, high, duplicate_high) = main
                .with_resolved_state_mut(owner, |state| {
                    state.push_number(1.0);
                    state.push_number(2.0);
                    state.push_number(3.0);
                    let low = state
                        .find_or_create_upvalue(0, gc)
                        .expect("low Upvalue is published");
                    let high = state
                        .find_or_create_upvalue(2, gc)
                        .expect("high Upvalue is published");
                    let middle = state
                        .find_or_create_upvalue(1, gc)
                        .expect("middle Upvalue is published");
                    let duplicate_high = state
                        .find_or_create_upvalue(2, gc)
                        .expect("same stack slot reuses its Upvalue");
                    assert_eq!(state.open_upvalues, [high, middle, low]);
                    (low, middle, high, duplicate_high)
                })
                .expect("owner state resolves");
            (owner, low, middle, high, duplicate_high)
        };

        assert_eq!(duplicate_high, high);
        let gc = runtime.heap.as_ref().get_ref().heap.collector();
        assert_eq!(
            gc.with_ref(low, Upvalue::open_location)
                .expect("low Upvalue remains registered"),
            Some((owner, 0))
        );
        assert_eq!(
            gc.with_ref(middle, Upvalue::open_location)
                .expect("middle Upvalue remains registered"),
            Some((owner, 1))
        );
        assert_eq!(
            gc.with_ref(high, Upvalue::open_location)
                .expect("high Upvalue remains registered"),
            Some((owner, 2))
        );
    }

    #[test]
    fn top_level_error_unwind_closes_root_frame_upvalues_before_reset() {
        let mut runtime = Runtime::new();
        let upvalue = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            main.push_number(42.0);
            let upvalue = main
                .find_or_create_upvalue(0, gc)
                .expect("root Upvalue is published");
            assert!(
                gc.with_ref(upvalue, Upvalue::is_open)
                    .expect("root Upvalue remains registered")
            );

            unwind_top_level_lua_error(main, gc).expect("top-level unwind succeeds");

            assert_eq!(main.current_ci, 0);
            assert!(main.open_upvalues.is_empty());
            upvalue
        };

        let collector = runtime.heap.as_ref().get_ref().heap.collector();
        let (closed, value) = collector
            .with_ref(upvalue, |upvalue| {
                (upvalue.is_closed(), upvalue.get_closed_value().clone())
            })
            .expect("closed root Upvalue remains registered");
        assert!(closed);
        assert_eq!(value, Value::Number(42.0));
    }

    #[test]
    fn owner_and_execution_guard_are_enforced() {
        let mut runtime = Runtime::new();
        let owner_thread = thread::current().id();
        assert_eq!(runtime.owner_thread_id(), owner_thread);
        assert_eq!(runtime.active_execution_count(), 0);

        {
            let parts = runtime.parts_mut().expect("owner may execute");
            assert_eq!(parts.active_execution_count(), 1);
        }
        assert_eq!(runtime.active_execution_count(), 0);

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || sender.send(thread::current().id()).unwrap())
            .join()
            .unwrap();
        runtime.owner_thread = receiver.recv().unwrap();
        assert!(matches!(
            runtime.check_owner(),
            Err(RuntimeAccessError::WrongThread { .. })
        ));
        runtime.owner_thread = owner_thread;
    }

    #[test]
    fn turn_driver_releases_before_switch_and_allows_handle_reentry() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let child = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };

        let mut turn_index = 0_usize;
        let completed = runtime
            .drive_state_turns(main, |handle, state| {
                assert_eq!(state.state_handle(), Some(handle));
                let nested_target = if handle == main { child } else { main };
                assert!(matches!(
                    state.with_resolved_state_mut(nested_target, |_| ()),
                    Err(StateResolveError::TurnBorrowActive {
                        slot,
                        generation,
                    }) if slot == handle.slot() && generation == handle.generation()
                ));

                let outcome = match turn_index {
                    0 => {
                        assert_eq!(handle, main);
                        RuntimeTurn::Switch(child)
                    }
                    1 => {
                        assert_eq!(handle, child);
                        RuntimeTurn::Switch(main)
                    }
                    2 => {
                        assert_eq!(handle, main);
                        RuntimeTurn::Complete("finished")
                    }
                    _ => panic!("driver executed an unexpected extra turn"),
                };
                turn_index += 1;
                outcome
            })
            .expect("all turn handles resolve");

        assert_eq!(completed, "finished");
        assert_eq!(turn_index, 3);
        assert_eq!(runtime.active_execution_count(), 0);
        let arena = runtime.heap.as_ref().get_ref().state_arena();
        assert_eq!(arena.peak_turn_borrowed_slots, 1);
        assert_eq!(
            arena.turn_borrow_events,
            vec![
                TurnBorrowEvent::Acquired(main),
                TurnBorrowEvent::Released(main),
                TurnBorrowEvent::Acquired(child),
                TurnBorrowEvent::Released(child),
                TurnBorrowEvent::Acquired(main),
                TurnBorrowEvent::Released(main),
            ]
        );
        assert!(arena.turn_borrow.is_none());
        assert!(arena.slots.iter().all(|slot| !slot.borrowed));
    }

    #[test]
    fn runtime_turn_can_scope_debug_access_to_an_owned_coroutine() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let child = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, gc, _) = parts.split_mut();
            gc.with_publication(|transaction| {
                let thread = transaction.alloc(Thread::new());
                state
                    .with_pending_coroutine_state(LuaState::new(), |pending, publisher| {
                        pending
                            .bind_thread(transaction, thread)
                            .expect("both State↔Thread edges bind");
                        pending.publish_thread_to_stack(publisher)
                    })
                    .expect("pending coroutine state is inserted")
                    .expect("bound Thread publishes to the main stack")
            })
        };

        runtime
            .drive_state_turns(main, |handle, state| {
                assert_eq!(handle, main);
                assert!(state.active_gc_ptr().is_some());
                state
                    .with_resolved_state_mut(child, |child| {
                        assert!(child.current_thread.is_some());
                        assert!(child.active_gc_ptr().is_some());
                        assert!(child.active_string_pool_ptr().is_some());
                    })
                    .expect("debug access resolves the inactive coroutine");
                RuntimeTurn::Complete(())
            })
            .expect("turn completes after scoped coroutine inspection");

        assert_eq!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .peak_turn_borrowed_slots,
            2
        );
    }

    #[test]
    fn turn_driver_restores_state_and_active_count_when_callback_panics() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");

        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = runtime.drive_state_turns::<()>(main, |_, _| panic!("intentional turn panic"));
        }));
        assert!(panic_result.is_err());
        assert_eq!(runtime.active_execution_count(), 0);
        {
            let arena = runtime.heap.as_ref().get_ref().state_arena();
            assert_eq!(
                arena.turn_borrow_events,
                vec![
                    TurnBorrowEvent::Acquired(main),
                    TurnBorrowEvent::Released(main)
                ]
            );
            assert!(arena.turn_borrow.is_none());
            assert!(arena.slots.iter().all(|slot| !slot.borrowed));
        }

        let result = runtime
            .drive_state_turns(main, |handle, _| {
                assert_eq!(handle, main);
                RuntimeTurn::Complete(7_u8)
            })
            .expect("the same handle can be borrowed after unwinding");
        assert_eq!(result, 7);
        assert_eq!(runtime.active_execution_count(), 0);
    }

    #[test]
    fn turn_driver_fails_closed_for_foreign_stale_and_borrowed_handles() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let foreign = Runtime::new();
        let foreign_handle = foreign
            .main_state_handle
            .expect("foreign main handle is registered");

        assert!(matches!(
            runtime.drive_state_turns(foreign_handle, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::ForeignRuntime { .. },
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);

        let stale = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };
        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena_mut()
            .remove_owned(stale)
            .expect("test child can be removed");
        assert!(matches!(
            runtime.drive_state_turns(stale, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::StaleGeneration { .. },
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);

        // SAFETY: RuntimeStorage is pinned and the test releases this synthetic
        // direct borrow before any teardown or further state access.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena_mut()
            .begin_direct_borrow(main)
            .expect("test can mark the main slot borrowed");
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::AlreadyBorrowed { .. },
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);
        // SAFETY: the same pinned arena owns the synthetic borrow above.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        assert!(heap.state_arena_mut().release(main));
        assert!(heap.state_arena().slots.iter().all(|slot| !slot.borrowed));
    }

    #[test]
    fn debug_upvalue_requests_convert_foreign_and_stale_owners_to_error_responses() {
        let mut runtime = Runtime::new();
        let requester = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let foreign_runtime = Runtime::new();
        let foreign_owner = foreign_runtime
            .main_state_handle
            .expect("foreign main state handle is registered");
        let stale_owner = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };
        let runtime_id = runtime.id;

        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena_mut()
            .remove_owned(stale_owner)
            .expect("test child can be removed");
        let arena =
            NonNull::new(heap.state_arena.get()).expect("pinned StateArena cell is non-null");
        let (gc, string_pool) = heap.heap.parts_mut();
        let upvalue = gc.create(Upvalue::new_closed(Value::Nil));

        for (owner, expected) in [
            (foreign_owner, "belongs to runtime"),
            (stale_owner, "generation is"),
        ] {
            let request = DebugUpvalueRequest {
                id: crate::native::NativeRequestId::new(1),
                requester,
                upvalue,
                owner,
                stack_index: 0,
                name: Value::Nil,
                operation: DebugUpvalueOperation::Read,
                protected: true,
                deferred: None,
            };
            let response =
                resolve_debug_upvalue_request(arena, gc, string_pool, runtime_id, &request);
            assert!(
                matches!(
                    response,
                    DebugUpvalueResponse::ErrorMessage(ref message)
                        if message.contains(expected)
                ),
                "owner resolution must become a protected Lua-visible response: {response:?}"
            );
        }
    }

    #[test]
    fn turn_driver_enforces_owner_and_running_phase_before_activation() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let owner = runtime.owner_thread_id();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || sender.send(thread::current().id()).unwrap())
            .join()
            .unwrap();
        runtime.owner_thread = receiver.recv().unwrap();

        assert!(matches!(
            runtime.drive_state_turns(main, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::WrongThread { .. })
        ));
        assert_eq!(runtime.active_execution_count(), 0);

        runtime.owner_thread = owner;
        runtime.active_executions = 1;
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::ActiveExecutions { count: 1, .. })
        ));
        assert_eq!(runtime.active_execution_count(), 1);
        runtime.active_executions = 0;

        runtime.close().expect("runtime closes");
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::NotRunning {
                phase: RuntimePhase::Closed,
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);
    }

    #[test]
    fn close_is_idempotent_and_reports_zeroed_heap() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert!(runtime.global_root().is_some());
        let initial_allocator = runtime.allocator_stats();
        {
            let mut parts = runtime.parts_mut().expect("main state is borrowable");
            let (state, _, _) = parts.split_mut();
            state.stack.ensure_space(2_048);
        }
        let grown_allocator = runtime.allocator_stats();
        assert!(grown_allocator.live_bytes > initial_allocator.live_bytes);
        assert!(grown_allocator.peak_bytes >= grown_allocator.live_bytes);

        let first = runtime.close().expect("first close succeeds");
        assert!(!first.already_closed);
        assert!(!first.heap_reclamation_deferred);
        assert_eq!(first.remaining_objects, 0);
        assert_eq!(first.remaining_roots, 0);
        assert_eq!(first.remaining_interned_strings, 0);
        assert_eq!(first.remaining_estimated_bytes, 0);
        assert_eq!(first.remaining_allocator_live_bytes, 0);
        assert!(first.allocator_peak_bytes >= grown_allocator.live_bytes);
        assert!(first.allocator_total_allocated_bytes >= first.allocator_peak_bytes);
        assert_eq!(first.remaining_coroutine_states, 0);
        assert_eq!(first.remaining_temporary_state_roots, 0);
        assert_eq!(first.rejected_temporary_state_root_releases, 0);
        assert_eq!(first.remaining_collector_queue_entries, 0);
        assert!(first.destroyed_objects >= 2);
        assert!(!first.lua_gc_callback_debt);
        assert_eq!(first.lua_finalizers_attempted, 0);
        assert_eq!(first.lua_finalizer_errors_ignored, 0);
        assert_eq!(first.runtime_resources_attempted, 0);
        assert_eq!(first.runtime_resources_drained, 0);
        assert_eq!(first.runtime_resources_already_closed, 0);
        assert_eq!(first.runtime_resources_stale, 0);
        assert_eq!(first.runtime_resource_drain_errors, 0);
        assert!(!first.io_resource_drain_debt);
        assert_eq!(runtime.phase(), RuntimePhase::Closed);
        assert!(runtime.main_state().is_none());
        assert!(runtime.main_state_handle.is_none());
        assert!(runtime.global_root().is_none());
        assert!(runtime.registry_root().is_none());

        let second = runtime.close().expect("second close is idempotent");
        assert!(second.already_closed);
        assert_eq!(second.destroyed_objects, first.destroyed_objects);
        assert_eq!(
            second.drained_coroutine_states,
            first.drained_coroutine_states
        );
        assert_eq!(second.remaining_objects, 0);
        assert_eq!(second.remaining_estimated_bytes, 0);
        assert_eq!(second.remaining_allocator_live_bytes, 0);
        assert_eq!(second.allocator_peak_bytes, first.allocator_peak_bytes);
        assert_eq!(second.remaining_temporary_state_roots, 0);
        assert_eq!(second.rejected_temporary_state_root_releases, 0);
        assert!(!second.heap_reclamation_deferred);
        assert!(matches!(
            runtime.parts_mut(),
            Err(RuntimeAccessError::NotRunning {
                phase: RuntimePhase::Closed,
                ..
            })
        ));
    }

    #[test]
    fn close_drains_runtime_resources_and_isolates_hook_panics() {
        RESOURCE_DRAIN_CALLS.store(0, Ordering::SeqCst);
        let mut runtime = Runtime::new();
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, gc, _) = parts.split_mut();
            assert!(
                state
                    .reserve_runtime_resource_registrations(2)
                    .expect("resource registry capacity is reserved")
            );
            let drained = gc.create(Userdata::new(0)).object_id();
            let panicking = gc.create(Userdata::new(0)).object_id();
            assert!(
                state
                    .register_runtime_resource(drained, count_resource_drain)
                    .expect("first resource registers")
            );
            assert!(
                state
                    .register_runtime_resource(panicking, panic_resource_drain)
                    .expect("second resource registers")
            );
        }

        let first = runtime.close().expect("resource hook panic is isolated");
        assert_eq!(RESOURCE_DRAIN_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(first.runtime_resources_attempted, 2);
        assert_eq!(first.runtime_resources_drained, 1);
        assert_eq!(first.runtime_resources_already_closed, 0);
        assert_eq!(first.runtime_resources_stale, 0);
        assert_eq!(first.runtime_resource_drain_errors, 1);
        assert_eq!(first.remaining_allocator_live_bytes, 0);
        assert!(!first.io_resource_drain_debt);

        let second = runtime.close().expect("closed Runtime remains idempotent");
        assert!(second.already_closed);
        assert_eq!(second.runtime_resources_attempted, 2);
        assert_eq!(RESOURCE_DRAIN_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn resource_registry_allocation_failure_is_transactional_and_retryable() {
        let mut runtime = Runtime::new();
        let before = runtime.allocator_stats();
        runtime.inject_allocator_failure_after(AllocationSite::RuntimeResourceRegistry, 0);
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            assert!(matches!(
                state.reserve_runtime_resource_registrations(1),
                Err(StateResolveError::Allocation(
                    ManagedAllocationError::Injected {
                        site: AllocationSite::RuntimeResourceRegistry,
                        ..
                    }
                ))
            ));
        }
        assert_eq!(runtime.allocator_stats(), before);

        {
            let mut parts = runtime.parts_mut().expect("retry parts are available");
            let (state, gc, _) = parts.split_mut();
            assert!(
                state
                    .reserve_runtime_resource_registrations(1)
                    .expect("one-shot allocation fault permits retry")
            );
            let resource = gc.create(Userdata::new(0)).object_id();
            assert!(
                state
                    .register_runtime_resource(resource, drain_resource_without_counter)
                    .expect("reserved retry registration succeeds")
            );
        }
        let report = runtime.close().expect("retry leaves Runtime closable");
        assert_eq!(report.runtime_resources_attempted, 1);
        assert_eq!(report.runtime_resources_drained, 1);
        assert_eq!(report.remaining_allocator_live_bytes, 0);
    }

    #[test]
    fn close_rejects_active_execution_without_changing_phase() {
        let mut runtime = Runtime::new();
        runtime.active_executions = 1;

        assert!(matches!(
            runtime.close(),
            Err(RuntimeAccessError::ActiveExecutions { count: 1, .. })
        ));
        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert!(runtime.main_state().is_some());

        runtime.active_executions = 0;
        runtime.close().expect("close succeeds once inactive");
    }

    #[test]
    fn close_preflight_rejects_free_list_corruption_before_mutation() {
        let mut runtime = Runtime::new();
        let removed = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };
        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena_mut()
            .remove_owned(removed)
            .expect("child is removed");
        heap.state_arena_mut().free_slots.push(removed.slot());

        assert!(matches!(
            runtime.close(),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::ArenaInvariant {
                    reason: "free-list contains a duplicate slot"
                },
                ..
            })
        ));
        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert!(runtime.main_state().is_some());
        assert!(runtime.global_root.is_some());

        // Repair the deliberately injected duplicate and prove normal shutdown
        // remains available after the fail-closed preflight.
        // SAFETY: RuntimeStorage remains pinned and the failed close did not
        // create an execution guard or mutate arena ownership.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        assert_eq!(
            heap.state_arena_mut().free_slots.pop(),
            Some(removed.slot())
        );
        runtime
            .close()
            .expect("close succeeds after test corruption is repaired");
    }

    #[test]
    fn shutdown_preflight_fault_preserves_arena_owned_main_state_for_retry() {
        PREFLIGHT_RESOURCE_DRAIN_CALLS.store(0, Ordering::SeqCst);
        let mut runtime = Runtime::new();
        let main_handle = runtime.main_state_handle.expect("main state handle");
        let main_pointer = runtime
            .main_state()
            .map(std::ptr::from_ref)
            .expect("main state is visible");
        let child_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            let child = main
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted");
            assert!(
                main.reserve_runtime_resource_registrations(1)
                    .expect("resource capacity is reserved")
            );
            let resource = gc.create(Userdata::new(0)).object_id();
            assert!(
                main.register_runtime_resource(resource, count_preflight_resource_drain)
                    .expect("resource registers")
            );
            child
        };
        let allocator_before = runtime.allocator_stats();

        runtime.inject_runtime_fault_after(RuntimeFaultSite::ShutdownPreflight, 0);
        assert!(matches!(
            runtime.close(),
            Err(RuntimeAccessError::InjectedFault {
                site: RuntimeFaultSite::ShutdownPreflight,
                ..
            })
        ));

        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert_eq!(runtime.main_state_handle, Some(main_handle));
        assert_eq!(
            runtime.main_state().map(std::ptr::from_ref),
            Some(main_pointer)
        );
        assert_eq!(runtime.live_coroutine_state_count(), 1);
        assert_eq!(runtime.allocator_stats(), allocator_before);
        assert_eq!(PREFLIGHT_RESOURCE_DRAIN_CALLS.load(Ordering::SeqCst), 0);
        let arena = runtime.heap.as_ref().get_ref().state_arena();
        assert_eq!(
            arena
                .validate_main(main_handle)
                .expect("fault leaves the main slot intact")
                .as_ptr(),
            main_pointer.cast_mut()
        );
        assert!(arena.validate(child_handle).is_ok());

        let report = runtime.close().expect("one-shot fault permits retry");
        assert_eq!(report.drained_coroutine_states, 1);
        assert_eq!(report.runtime_resources_drained, 1);
        assert_eq!(PREFLIGHT_RESOURCE_DRAIN_CALLS.load(Ordering::SeqCst), 1);
        assert!(runtime.main_state().is_none());
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .validate(main_handle),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn arena_rejects_current_foreign_and_nested_active_states() {
        let mut runtime = Runtime::new();
        let main_handle = runtime
            .main_state_handle
            .expect("main handle is registered");
        let child_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            assert!(matches!(
                state.with_resolved_state_mut(main_handle, |_| ()),
                Err(StateResolveError::CurrentState)
            ));
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };
        assert_eq!(runtime.live_coroutine_state_count(), 1);

        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            let nested = state
                .with_resolved_state_mut(child_handle, |child| {
                    child.with_resolved_state_mut(main_handle, |_| ())
                })
                .expect("child itself resolves");
            assert!(matches!(
                nested,
                Err(StateResolveError::AlreadyBorrowed { .. })
            ));
        }

        let mut foreign_runtime = Runtime::new();
        let mut foreign_parts = foreign_runtime
            .parts_mut()
            .expect("foreign runtime parts are available");
        let (foreign_state, _, _) = foreign_parts.split_mut();
        assert!(matches!(
            foreign_state.with_resolved_state_mut(child_handle, |_| ()),
            Err(StateResolveError::ForeignRuntime { .. })
        ));
    }

    #[test]
    fn removed_state_handles_become_stale_and_slots_advance_generation() {
        let mut runtime = Runtime::new();
        let first_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("first child is inserted")
        };

        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena_mut()
            .remove_owned(first_handle)
            .expect("owned child can be removed");
        assert_eq!(heap.state_arena().live_coroutine_state_count(), 0);

        let second_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("second child is inserted")
        };
        assert_eq!(second_handle.slot(), first_handle.slot());
        assert_ne!(second_handle.generation(), first_handle.generation());

        let mut parts = runtime.parts_mut().expect("parts are available");
        let (state, _, _) = parts.split_mut();
        assert!(matches!(
            state.with_resolved_state_mut(first_handle, |_| ()),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn generation_max_is_issued_once_then_the_slot_is_permanently_retired() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let original = arena.insert_owned(Box::new(LuaState::new()));
        let slot_index = original.slot();

        arena.slots[slot_index].generation = u64::MAX - 1;
        let penultimate = arena.issue_handle(slot_index);
        arena
            .remove_owned(penultimate)
            .expect("penultimate generation can be removed");

        assert_eq!(arena.slots[slot_index].generation, u64::MAX);
        assert!(!arena.slots[slot_index].retired);
        assert_eq!(
            arena
                .free_slots
                .iter()
                .filter(|&&slot| slot == slot_index)
                .count(),
            1
        );

        let final_handle = arena.insert_owned(Box::new(LuaState::new()));
        assert_eq!(final_handle.slot(), slot_index);
        assert_eq!(final_handle.generation(), u64::MAX);
        assert!(matches!(
            arena.validate(penultimate),
            Err(StateResolveError::StaleGeneration {
                requested,
                actual,
                ..
            }) if requested == u64::MAX - 1 && actual == u64::MAX
        ));

        arena
            .remove_owned(final_handle)
            .expect("final generation can be removed exactly once");
        assert!(arena.slots[slot_index].retired);
        assert!(arena.slots[slot_index].state.is_none());
        assert!(!arena.free_slots.contains(&slot_index));
        assert!(matches!(
            arena.validate(final_handle),
            Err(StateResolveError::RetiredSlot {
                slot,
                generation: u64::MAX
            }) if slot == slot_index
        ));
        assert!(matches!(
            arena.validate(original),
            Err(StateResolveError::RetiredSlot { .. })
        ));

        let replacement = arena.insert_owned(Box::new(LuaState::new()));
        assert_ne!(replacement.slot(), slot_index);
        assert_eq!(replacement.generation(), 1);
        assert_eq!(arena.live_coroutine_state_count(), 1);
        arena
            .remove_owned(replacement)
            .expect("replacement state can be removed");
        assert_eq!(arena.live_coroutine_state_count(), 0);
    }

    #[test]
    fn pending_state_rollback_retires_generation_max_exactly_once() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let reusable = arena.insert_owned(Box::new(LuaState::new()));
        arena
            .remove_owned(reusable)
            .expect("test slot becomes reusable");
        let slot_index = reusable.slot();
        arena.slots[slot_index].generation = u64::MAX;

        let (handle, temporary_root_id) = arena
            .insert_pending_owned(Box::new(LuaState::new()), None)
            .expect("final generation can host one pending state");
        assert_eq!(handle.slot(), slot_index);
        assert_eq!(handle.generation(), u64::MAX);
        assert_eq!(arena.temporary_state_root_count(), 1);
        assert!(matches!(
            arena.rollback_pending(handle, temporary_root_id + 1),
            Err(PendingStateError::RootMismatch)
        ));
        assert_eq!(arena.temporary_state_root_count(), 1);
        assert_eq!(arena.live_coroutine_state_count(), 1);

        let arena_ptr = NonNull::from(&mut arena);
        {
            let mut publisher = LuaState::new();
            let _pending = PendingState {
                arena: arena_ptr,
                publisher: NonNull::from(&mut publisher),
                handle,
                temporary_root_id,
                bound_thread: None,
                committed: false,
                _scope: PhantomData,
            };
        }

        let slot = &arena.slots[slot_index];
        assert!(slot.retired);
        assert_eq!(slot.generation, u64::MAX);
        assert!(slot.state.is_none());
        assert!(!arena.free_slots.contains(&slot_index));
        assert_eq!(arena.temporary_state_root_count(), 0);
        assert_eq!(arena.live_coroutine_state_count(), 0);
        assert_eq!(arena.rejected_temporary_root_releases, 0);
        assert!(matches!(
            arena.validate(handle),
            Err(StateResolveError::RetiredSlot {
                slot,
                generation: u64::MAX
            }) if slot == slot_index
        ));

        let replacement = arena.insert_owned(Box::new(LuaState::new()));
        assert_ne!(replacement.slot(), slot_index);
    }

    #[test]
    fn duplicate_or_occupied_free_list_entries_cannot_overwrite_a_live_slot() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let first = arena.insert_owned(Box::new(LuaState::new()));
        arena
            .remove_owned(first)
            .expect("first state can be removed");

        let recycled_slot = first.slot();
        arena.free_slots.push(recycled_slot);
        let recycled = arena.insert_owned(Box::new(LuaState::new()));
        assert_eq!(recycled.slot(), recycled_slot);

        let other = arena.insert_owned(Box::new(LuaState::new()));
        assert_ne!(other.slot(), recycled_slot);
        assert_eq!(arena.live_coroutine_state_count(), 2);
        assert!(arena.validate(recycled).is_ok());
        assert!(arena.validate(other).is_ok());

        arena
            .remove_owned(recycled)
            .expect("recycled state can be removed");
        arena
            .remove_owned(other)
            .expect("other state can be removed");
        assert_eq!(arena.live_coroutine_state_count(), 0);
    }

    #[test]
    fn arena_releases_one_thousand_owned_states_back_to_baseline() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let baseline = arena.live_coroutine_state_count();
        let mut first_handle = None;
        let mut previous_generation = 0;

        for _ in 0..1_000 {
            let handle = arena.insert_owned(Box::new(LuaState::new()));
            first_handle.get_or_insert(handle);
            assert!(handle.generation() > previous_generation);
            previous_generation = handle.generation();
            assert_eq!(arena.live_coroutine_state_count(), baseline + 1);
            arena
                .remove_owned(handle)
                .expect("an occupied owned slot can be released exactly once");
            assert_eq!(arena.live_coroutine_state_count(), baseline);
            assert!(matches!(
                arena.validate(handle),
                Err(StateResolveError::StaleGeneration { .. })
            ));
            assert!(matches!(
                arena.validate(first_handle.expect("first handle is recorded")),
                Err(StateResolveError::StaleGeneration { .. })
            ));
        }
    }

    #[test]
    fn close_drains_coroutine_states_and_invalidates_their_generations() {
        let mut runtime = Runtime::new();
        let child_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };

        let report = runtime.close().expect("runtime closes");
        assert_eq!(report.drained_coroutine_states, 1);
        assert_eq!(report.remaining_coroutine_states, 0);
        assert_eq!(report.remaining_temporary_state_roots, 0);
        assert_eq!(report.rejected_temporary_state_root_releases, 0);
        assert_eq!(runtime.live_coroutine_state_count(), 0);
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .validate(child_handle),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn arena_drain_closes_open_upvalue_before_invalidating_owner_handle() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer, AllocationLedger::new());
        let mut gc = GarbageCollector::new();
        let owner = arena.insert_owned(Box::new(LuaState::new()));
        let arena_ptr = NonNull::from(&mut arena);
        let upvalue = StateArena::with_turn_state_mut(arena_ptr, owner, |state| {
            state.push_number(42.0);
            state
                .find_or_create_upvalue(0, &mut gc)
                .expect("owned state publishes its Upvalue")
        })
        .expect("owner state turn succeeds");

        let report = arena.drain_owned(&mut gc).expect("owned state drains");

        assert_eq!(report.drained_coroutine_states, 1);
        assert_eq!(report.drained_main_states, 0);
        assert_eq!(report.state_shutdown.closed_open_upvalues, 1);
        assert_eq!(
            gc.with_ref(upvalue, |upvalue| {
                (upvalue.open_location(), upvalue.get_closed_value().clone())
            })
            .expect("closed Upvalue remains registered"),
            (None, Value::Number(42.0))
        );
        assert!(matches!(
            arena.validate(owner),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn arena_drain_orders_coroutines_before_its_owned_main_state() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut storage = Box::pin(RuntimeStorage::new(issuer));
        // SAFETY: RuntimeStorage is pinned for every arena owner pointer below.
        let storage = unsafe { Pin::get_unchecked_mut(storage.as_mut()) };
        let arena_cell = storage.state_arena_cell();
        let main = storage
            .state_arena_mut()
            .insert_main_owned(Box::new(LuaState::new()), arena_cell)
            .expect("main state enters the arena");
        let first = storage
            .state_arena_mut()
            .insert_owned(Box::new(LuaState::new()));
        let second = storage
            .state_arena_mut()
            .insert_owned(Box::new(LuaState::new()));

        // SAFETY: the arena and collector are disjoint pinned storage fields,
        // and no state borrow is active.
        let report = unsafe { &mut *storage.state_arena.get() }
            .drain_owned(storage.heap.collector_mut())
            .expect("all states drain");

        assert_eq!(report.drained_coroutine_states, 2);
        assert_eq!(report.drained_main_states, 1);
        assert_eq!(report.drained_state_handles, vec![first, second, main]);
        assert_eq!(storage.state_arena().live_coroutine_state_count(), 0);
        for handle in [first, second, main] {
            assert!(matches!(
                storage.state_arena().validate(handle),
                Err(StateResolveError::StaleGeneration { .. })
            ));
        }
    }

    #[test]
    fn close_retires_a_coroutine_at_generation_max_without_reuse_or_wrap() {
        let mut runtime = Runtime::new();
        let child_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };

        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        let arena_cell = heap.state_arena_cell();
        let arena = heap.state_arena_mut();
        let slot_index = child_handle.slot();
        arena.slots[slot_index].generation = u64::MAX;
        let final_handle = arena.issue_handle(slot_index);
        let mut child_state = arena.slots[slot_index]
            .state
            .expect("child slot remains occupied");
        // SAFETY: the slot owns this live Box and the test has exclusive
        // Runtime access; this only synchronizes its diagnostic handle with
        // the forced boundary generation.
        unsafe {
            child_state
                .as_mut()
                .attach_runtime_state(final_handle, Some(arena_cell));
        }

        let first = runtime.close().expect("runtime closes at generation max");
        assert_eq!(first.drained_coroutine_states, 1);
        let slot = &runtime.heap.as_ref().get_ref().state_arena().slots[slot_index];
        assert!(slot.retired);
        assert_eq!(slot.generation, u64::MAX);
        assert!(slot.state.is_none());
        assert!(
            !runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .free_slots
                .contains(&slot_index)
        );
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena()
                .validate(final_handle),
            Err(StateResolveError::RetiredSlot { .. })
        ));

        let second = runtime.close().expect("second close is idempotent");
        assert!(second.already_closed);
        assert_eq!(
            runtime.heap.as_ref().get_ref().state_arena().slots[slot_index].generation,
            u64::MAX
        );
    }

    #[test]
    fn close_closes_main_and_coroutine_open_upvalues_before_heap_teardown() {
        let mut runtime = Runtime::new();
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            main.push_value(lua_core::value::Value::Number(1.0));
            main.find_or_create_upvalue(0, gc)
                .expect("main state publishes an open Upvalue");

            let child_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted");
            main.with_resolved_state_mut(child_handle, |child| {
                child.push_value(lua_core::value::Value::Number(2.0));
                child
                    .find_or_create_upvalue(0, gc)
                    .expect("child state publishes an open Upvalue");
            })
            .expect("child resolves");
        }

        let report = runtime.close().expect("runtime closes");

        assert_eq!(report.closed_open_upvalues, 2);
        assert_eq!(report.rejected_open_upvalue_edges, 0);
        assert_eq!(report.open_upvalue_owner_mismatches, 0);
        assert_eq!(report.open_upvalue_stack_values_missing, 0);
        assert_eq!(report.remaining_objects, 0);
    }

    #[test]
    fn close_reports_invalid_open_upvalue_without_dereference() {
        let mut runtime = Runtime::new();
        let mut foreign_gc = GarbageCollector::new();
        let foreign_upvalue = foreign_gc.create(lua_core::upvalue::Upvalue::new_closed(
            lua_core::value::Value::Nil,
        ));
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, _, _) = parts.split_mut();
            main.open_upvalues.push(foreign_upvalue);
        }

        let report = runtime.close().expect("runtime closes fail-closed");

        assert_eq!(report.rejected_open_upvalue_edges, 1);
        assert_eq!(report.closed_open_upvalues, 0);
        assert_eq!(report.remaining_objects, 0);
        assert!(!report.heap_reclamation_deferred);

        let mut foreign_pool = StringPool::new();
        foreign_gc.destroy_all(&mut foreign_pool);
    }

    #[test]
    fn runtime_drop_uses_the_same_destroy_all_path_for_fixed_and_ordinary_userdata() {
        unsafe fn count_probe(payload: *mut u8) {
            let mut encoded = [0_u8; std::mem::size_of::<usize>()];
            // SAFETY: probe_userdata stores exactly one native-endian usize.
            unsafe {
                std::ptr::copy_nonoverlapping(payload, encoded.as_mut_ptr(), encoded.len());
            }
            let counter = usize::from_ne_bytes(encoded) as *const AtomicUsize;
            // SAFETY: both counters outlive the scoped Runtime drop.
            unsafe {
                (*counter).fetch_add(1, Ordering::SeqCst);
            }
        }

        fn probe_userdata(counter: &AtomicUsize) -> Userdata {
            let mut userdata = Userdata::new(std::mem::size_of::<usize>());
            userdata
                .data_mut()
                .copy_from_slice(&(std::ptr::from_ref(counter) as usize).to_ne_bytes());
            // SAFETY: count_probe reads only the encoded pointer synchronously
            // during Runtime Drop and does not retain it.
            unsafe {
                userdata.set_destructor(count_probe);
            }
            userdata
        }

        let ordinary_drops = AtomicUsize::new(0);
        let fixed_drops = AtomicUsize::new(0);
        {
            let mut runtime = Runtime::new();
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (_, gc, _) = parts.split_mut();
            gc.create(probe_userdata(&ordinary_drops));
            let fixed = gc.create(probe_userdata(&fixed_drops));
            gc.fix(fixed)
                .expect("fixed userdata belongs to this collector");
        }

        assert_eq!(ordinary_drops.load(Ordering::SeqCst), 1);
        assert_eq!(fixed_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn one_thousand_runtime_coroutine_close_cycles_reach_zero() {
        for _ in 0..1_000 {
            let mut runtime = Runtime::new();
            {
                let mut parts = runtime.parts_mut().expect("parts are available");
                let (main, _, _) = parts.split_mut();
                main.insert_coroutine_state(LuaState::new())
                    .expect("child is inserted");
            }

            let report = runtime.close().expect("runtime closes");
            assert!(!report.heap_reclamation_deferred);
            assert_eq!(report.remaining_objects, 0);
            assert_eq!(report.remaining_roots, 0);
            assert_eq!(report.remaining_interned_strings, 0);
            assert_eq!(report.remaining_estimated_bytes, 0);
            assert_eq!(report.remaining_coroutine_states, 0);
            assert_eq!(report.remaining_temporary_state_roots, 0);
            assert_eq!(report.rejected_temporary_state_root_releases, 0);
            assert_eq!(report.remaining_collector_queue_entries, 0);
            assert_eq!(report.drained_coroutine_states, 1);
        }
    }
}

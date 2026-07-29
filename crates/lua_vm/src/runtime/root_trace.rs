//! Runtime-owned, mark-only root traversal.
//!
//! This is intentionally a live-set diagnostic, not a collector cycle. It
//! mutates only mark colors/work queues and never calls destructive sweep,
//! collection, finalization, or shutdown APIs.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::pin::Pin;

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::state_handle::{RuntimeId, StateHandle};
use lua_core::table::Table;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use crate::state::LuaState;

use super::{
    NativeActivationStack, Runtime, RuntimeAccessError, RuntimePhase, StateArena, StateResolveError,
};

/// Inventory-aligned root kinds covered by this mark-only slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RuntimeRootKind {
    /// Roots explicitly registered with the collector.
    CollectorExplicitRoot,
    /// Runtime and state global-table handles.
    GlobalTable,
    /// Per-state thread and chunk environments.
    GlobalEnvironments,
    /// Runtime registry and per-state registry handles.
    Registry,
    /// Installed nil/boolean/number metatables.
    PrimitiveMetatables,
    /// Runtime-owned main StateArena entry.
    MainStateEntry,
    /// The current/running Thread recorded by a reachable state.
    RunningThread,
    /// Main-state active stack window.
    MainStack,
    /// Reachable coroutine state edge or active stack window.
    CoroutineStack,
    /// Function slot of an active CallInfo.
    CallFunction,
    /// Managed Proto handle stored by an active CallInfo.
    ActiveProto,
    /// Proto retained by one-shot debug line suppression state.
    DebugProto,
    /// Vararg snapshot of an active CallInfo.
    CallVarargs,
    /// Open-Upvalue list head owned by a reachable state.
    OpenUpvalues,
    /// Caller Thread edge traced by the object graph.
    ThreadCallerChain,
    /// Debug hook Value owned by a reachable state.
    DebugHook,
    /// Values retained across coroutine yield.
    YieldedValues,
    /// Last error Value retained by a state.
    LastError,
    /// Runtime-owned deferred coroutine requests, snapshots, and responses.
    CoroutineActivationBuffer,
    /// Collector-owned userdata awaiting protected finalizer delivery.
    PendingFinalizers,
    /// Lexical object-publication roots.
    TemporaryProtectedRoots,
    /// Lexical PendingState handles not yet reachable through a Thread.
    TemporaryStateRoots,
    /// Runtime-owned fixed metamethod and emergency strings.
    FixedStrings,
}

impl RuntimeRootKind {
    /// Return the exact name used by `gc_root_inventory.json`.
    pub const fn inventory_name(self) -> &'static str {
        match self {
            Self::CollectorExplicitRoot => "COLLECTOR_EXPLICIT_ROOT",
            Self::GlobalTable => "GLOBAL_TABLE",
            Self::GlobalEnvironments => "GLOBAL_ENVIRONMENTS",
            Self::Registry => "REGISTRY",
            Self::PrimitiveMetatables => "PRIMITIVE_METATABLES",
            Self::MainStateEntry => "MAIN_STATE_ENTRY",
            Self::RunningThread => "RUNNING_THREAD",
            Self::MainStack => "MAIN_STACK",
            Self::CoroutineStack => "COROUTINE_STACK",
            Self::CallFunction => "CALL_FUNCTION",
            Self::ActiveProto => "ACTIVE_PROTO",
            Self::DebugProto => "DEBUG_PROTO",
            Self::CallVarargs => "CALL_VARARGS",
            Self::OpenUpvalues => "OPEN_UPVALUES",
            Self::ThreadCallerChain => "THREAD_CALLER_CHAIN",
            Self::DebugHook => "DEBUG_HOOK",
            Self::YieldedValues => "YIELDED_VALUES",
            Self::LastError => "LAST_ERROR",
            Self::CoroutineActivationBuffer => "COROUTINE_ACTIVATION_BUFFER",
            Self::PendingFinalizers => "PENDING_FINALIZERS",
            Self::TemporaryProtectedRoots => "TEMPORARY_PROTECTED_ROOTS",
            Self::TemporaryStateRoots => "TEMPORARY_STATE_ROOTS",
            Self::FixedStrings => "FIXED_STRINGS",
        }
    }
}

/// Number of scanned edges attributed to one root kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootEdgeCount {
    /// Inventory-aligned kind.
    pub kind: RuntimeRootKind,
    /// Number of root edges scanned. Duplicate object identities still count
    /// as separate owner edges.
    pub edges: usize,
}

/// State-handle validation failure observed while reaching a fixed point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTraceFailure {
    /// Handle published by a Runtime root or reachable Thread.
    pub handle: StateHandle,
    /// Deterministic arena validation result.
    pub error: StateResolveError,
}

/// Unsafe ownership gaps that mark-only diagnostics report but never
/// dereference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnsafeTraceGapKind {
    /// LuaState's logical top exceeded Stack's initialized window.
    StackWindowOutOfBounds {
        /// LuaState logical top.
        top: usize,
        /// Stack initialized size.
        available: usize,
    },
    /// `current_ci` named a frame beyond the CallInfo vector.
    ActiveCallStackOutOfBounds {
        /// Requested active CallInfo index.
        current_ci: usize,
        /// Number of allocated CallInfo records.
        available: usize,
    },
    /// An active CallInfo function index did not name an initialized stack
    /// slot.
    CallFunctionOutOfBounds {
        /// Active frame index.
        frame: usize,
        /// Function stack index.
        function_index: usize,
    },
}

/// One explicit unsafe gap associated with a validated state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnsafeTraceGap {
    /// State containing the unresolved representation.
    pub state: StateHandle,
    /// Representation that was intentionally not dereferenced.
    pub kind: UnsafeTraceGapKind,
}

/// A copied GC edge that did not name a registered object of the expected
/// concrete type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnresolvedObjectEdge {
    /// State that owned the edge, or `None` for a Runtime-level root.
    pub state: Option<StateHandle>,
    /// Inventory-aligned owner kind.
    pub kind: RuntimeRootKind,
}

/// Machine-assertable result of a non-destructive Runtime root traversal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkOnlyReport {
    /// Runtime that owned the traversal.
    pub runtime_id: RuntimeId,
    /// Total collector objects before and after traversal.
    pub total_objects: usize,
    /// Objects reached and colored gray/black.
    pub marked_objects: usize,
    /// Concrete GC objects popped from the gray queue.
    pub object_trace_steps: usize,
    /// Validated states whose snapshots were scanned.
    pub traced_states: Vec<StateHandle>,
    /// Invalid state edges, sorted by handle.
    pub failed_state_handles: Vec<StateTraceFailure>,
    /// Explicit collector roots that were still registered.
    pub collector_roots_seeded: usize,
    /// Explicit collector roots rejected without dereference.
    pub collector_roots_rejected: usize,
    /// Object-graph child edges rejected before dereference because they did
    /// not belong to this Runtime's collector.
    pub rejected_child_edges: usize,
    /// Inventory-aligned edge counts, sorted by kind.
    pub root_edges: Vec<RootEdgeCount>,
    /// Raw-pointer/incomplete-ownership gaps, sorted deterministically.
    pub unsafe_gaps: Vec<UnsafeTraceGap>,
    /// Invalid copied GC edges, sorted deterministically.
    pub unresolved_object_edges: Vec<UnresolvedObjectEdge>,
    /// Gray work remaining at report time; a completed fixed point is zero.
    pub pending_objects: usize,
}

impl MarkOnlyReport {
    /// Return the number of scanned edges for one inventory root kind.
    pub fn root_edge_count(&self, kind: RuntimeRootKind) -> usize {
        self.root_edges
            .iter()
            .find(|entry| entry.kind == kind)
            .map_or(0, |entry| entry.edges)
    }
}

enum RootObject {
    Value(Value),
    Proto(GcRef<Proto>),
    Upvalue(GcRef<Upvalue>),
}

struct RootEdge {
    kind: RuntimeRootKind,
    object: RootObject,
}

#[derive(Default)]
struct StateSnapshot {
    edges: Vec<RootEdge>,
    gaps: Vec<UnsafeTraceGap>,
}

impl StateSnapshot {
    fn push_value(&mut self, kind: RuntimeRootKind, value: Value) {
        self.edges.push(RootEdge {
            kind,
            object: RootObject::Value(value),
        });
    }

    fn push_table(&mut self, kind: RuntimeRootKind, table: Option<GcRef<Table>>) {
        if let Some(table) = table {
            self.push_value(kind, Value::Table(table));
        }
    }

    fn push_proto(&mut self, kind: RuntimeRootKind, proto: GcRef<Proto>) {
        self.edges.push(RootEdge {
            kind,
            object: RootObject::Proto(proto),
        });
    }
}

impl Runtime {
    /// Trace the Runtime-owned live set without destroying any state or object.
    ///
    /// The caller must be on the owner thread, the Runtime must be running, and
    /// no `RuntimePartsMut` execution borrow may be active. The algorithm
    /// alternates collector-object and validated StateHandle queues until both
    /// are empty. Merely occupying a coroutine arena slot does not make that
    /// state reachable.
    pub fn trace_roots_mark_only(&mut self) -> Result<MarkOnlyReport, RuntimeAccessError> {
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

        let main_handle =
            self.main_state_handle
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;
        let global_root = self.global_root;
        let registry_root = self.registry_root;
        let fixed_strings = self.fixed_strings.clone();

        // SAFETY: RuntimeStorage is pinned, Runtime is exclusively borrowed, and
        // the active-execution check establishes U-03 for immutable state
        // snapshots alongside mutable collector mark state.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let state_arena = &mut heap.state_arena;
        let native_activations = &mut heap.native_activations;
        let gc = heap.heap.collector_mut();
        Ok(trace_roots_mark_only_at_safe_point(
            RuntimeRootSet {
                runtime_id: self.id,
                main_handle,
                global_root,
                registry_root,
                fixed_strings: &fixed_strings,
            },
            state_arena,
            native_activations,
            gc,
        ))
    }
}

/// Immutable Runtime-owned roots consumed by one stop-the-world trace.
#[derive(Clone, Copy)]
pub(super) struct RuntimeRootSet<'a> {
    pub(super) runtime_id: RuntimeId,
    pub(super) main_handle: StateHandle,
    pub(super) global_root: Option<GcRef<Table>>,
    pub(super) registry_root: Option<GcRef<Table>>,
    pub(super) fixed_strings: &'a [GcRef<GcString>],
}

/// Persistent half of an incremental Runtime root traversal.
///
/// Collector object colors/work live in `GarbageCollector`; this structure
/// retains the separate validated `StateHandle` queue and diagnostics between
/// public `collectgarbage("step")` calls.
pub(super) struct IncrementalRootTrace {
    runtime_id: RuntimeId,
    main_handle: StateHandle,
    total_objects: usize,
    collector_roots_seeded: usize,
    collector_roots_rejected: usize,
    root_counts: BTreeMap<RuntimeRootKind, usize>,
    state_queue: VecDeque<StateHandle>,
    attempted_states: HashSet<StateHandle>,
    traced_states: Vec<StateHandle>,
    failed_state_handles: Vec<StateTraceFailure>,
    unsafe_gaps: Vec<UnsafeTraceGap>,
    unresolved_object_edges: Vec<UnresolvedObjectEdge>,
    object_trace_steps: usize,
    collector_cycle_serial: u64,
}

impl IncrementalRootTrace {
    /// Seed one new Runtime/collector incremental cycle.
    pub(super) fn begin(
        roots: RuntimeRootSet<'_>,
        state_arena: &StateArena,
        native_activations: &NativeActivationStack,
        gc: &mut GarbageCollector,
    ) -> Self {
        debug_assert!(state_arena.turn_borrow.is_none());
        let total_objects = gc.object_count();
        let seed_report = gc.begin_incremental_mark();
        let mut trace = Self {
            runtime_id: roots.runtime_id,
            main_handle: roots.main_handle,
            total_objects,
            collector_roots_seeded: seed_report.seeded,
            collector_roots_rejected: seed_report.rejected,
            root_counts: BTreeMap::new(),
            state_queue: VecDeque::new(),
            attempted_states: HashSet::new(),
            traced_states: Vec::new(),
            failed_state_handles: Vec::new(),
            unsafe_gaps: Vec::new(),
            unresolved_object_edges: Vec::new(),
            object_trace_steps: 0,
            collector_cycle_serial: gc.incremental_cycle_serial(),
        };
        increment(
            &mut trace.root_counts,
            RuntimeRootKind::CollectorExplicitRoot,
            seed_report
                .seeded
                .saturating_sub(seed_report.temporary_seeded)
                .saturating_sub(seed_report.pending_finalizers_seeded),
        );
        increment(
            &mut trace.root_counts,
            RuntimeRootKind::TemporaryProtectedRoots,
            seed_report.temporary_seeded,
        );
        increment(
            &mut trace.root_counts,
            RuntimeRootKind::PendingFinalizers,
            seed_report.pending_finalizers_seeded,
        );
        trace.seed_runtime_snapshot(roots, state_arena, native_activations, gc, false);
        trace
    }

    /// Whether this state queue still belongs to the collector's active cycle.
    pub(super) fn matches_collector(&self, gc: &GarbageCollector) -> bool {
        self.collector_cycle_serial == gc.incremental_cycle_serial()
    }

    /// Re-scan all known Runtime roots at the atomic boundary.
    pub(super) fn atomic_rescan(
        &mut self,
        roots: RuntimeRootSet<'_>,
        state_arena: &StateArena,
        native_activations: &NativeActivationStack,
        gc: &mut GarbageCollector,
    ) {
        self.attempted_states.clear();
        self.seed_runtime_snapshot(roots, state_arena, native_activations, gc, true);
    }

    fn seed_runtime_snapshot(
        &mut self,
        roots: RuntimeRootSet<'_>,
        state_arena: &StateArena,
        native_activations: &NativeActivationStack,
        gc: &mut GarbageCollector,
        include_known_states: bool,
    ) {
        let fixed_strings_seeded = roots
            .fixed_strings
            .iter()
            .copied()
            .filter(|string| gc.mark_registered(*string))
            .count();
        increment(
            &mut self.root_counts,
            RuntimeRootKind::FixedStrings,
            fixed_strings_seeded,
        );
        increment(
            &mut self.root_counts,
            RuntimeRootKind::CoroutineActivationBuffer,
            native_activations.frames.len()
                + native_activations.upvalue_transfers.len()
                + native_activations.gc_frames.len(),
        );
        native_activations.seed_roots(gc);
        mark_runtime_table(
            gc,
            roots.global_root,
            RuntimeRootKind::GlobalTable,
            &mut self.root_counts,
            &mut self.unresolved_object_edges,
        );
        mark_runtime_table(
            gc,
            roots.registry_root,
            RuntimeRootKind::Registry,
            &mut self.root_counts,
            &mut self.unresolved_object_edges,
        );

        let temporary_state_roots = state_arena.temporary_state_roots();
        increment(
            &mut self.root_counts,
            RuntimeRootKind::TemporaryStateRoots,
            temporary_state_roots.len(),
        );
        self.state_queue.push_back(roots.main_handle);
        self.state_queue.extend(temporary_state_roots);
        if include_known_states {
            self.state_queue.extend(self.traced_states.iter().copied());
        }
        increment(&mut self.root_counts, RuntimeRootKind::MainStateEntry, 1);
    }

    /// Consume at most `budget` validated state snapshots or gray objects.
    pub(super) fn step(
        &mut self,
        state_arena: &StateArena,
        gc: &mut GarbageCollector,
        budget: usize,
    ) -> usize {
        debug_assert!(state_arena.turn_borrow.is_none());
        let mut completed = 0usize;
        let budget = budget.max(1);
        while completed < budget {
            let mut traced_state = false;
            while let Some(handle) = self.state_queue.pop_front() {
                if !self.attempted_states.insert(handle) {
                    continue;
                }
                match state_arena.resolve_for_trace(handle) {
                    Ok(state_pointer) => {
                        // SAFETY: StateArena validates the handle and the
                        // scheduler has released its turn borrow.
                        let snapshot = unsafe {
                            snapshot_state(
                                state_pointer.as_ref(),
                                handle,
                                handle == self.main_handle,
                            )
                        };
                        self.traced_states.push(handle);
                        self.unsafe_gaps.extend(snapshot.gaps);
                        mark_snapshot(
                            gc,
                            handle,
                            snapshot.edges,
                            &mut self.root_counts,
                            &mut self.unresolved_object_edges,
                        );
                    }
                    Err(error) => self
                        .failed_state_handles
                        .push(StateTraceFailure { handle, error }),
                }
                completed = completed.saturating_add(1);
                traced_state = true;
                break;
            }
            if traced_state {
                continue;
            }

            let Some(step) = gc.propagate_one_marked_object() else {
                break;
            };
            self.object_trace_steps = self.object_trace_steps.saturating_add(1);
            if step.traced_thread_caller {
                increment(&mut self.root_counts, RuntimeRootKind::ThreadCallerChain, 1);
            }
            if let Some(handle) = step.thread_state_handle {
                increment(&mut self.root_counts, RuntimeRootKind::CoroutineStack, 1);
                self.state_queue.push_back(handle);
            }
            if let Some(handle) = step.upvalue_state_handle {
                increment(&mut self.root_counts, RuntimeRootKind::OpenUpvalues, 1);
                self.state_queue.push_back(handle);
            }
            completed = completed.saturating_add(1);
        }
        completed
    }

    /// Drain all remaining atomic work.
    pub(super) fn drain(&mut self, state_arena: &StateArena, gc: &mut GarbageCollector) {
        while !self.is_complete(gc) {
            let budget = gc
                .object_count()
                .saturating_add(self.state_queue.len())
                .saturating_add(1);
            if self.step(state_arena, gc, budget) == 0 {
                break;
            }
        }
    }

    /// Whether both the state and collector work queues are empty.
    pub(super) fn is_complete(&self, gc: &GarbageCollector) -> bool {
        self.state_queue.is_empty() && gc.pending_mark_count() == 0
    }

    /// Current sweep-safe diagnostics.
    pub(super) fn report(&self, gc: &GarbageCollector) -> MarkOnlyReport {
        let mut traced_states = self.traced_states.clone();
        traced_states.sort_unstable();
        traced_states.dedup();
        let mut failed_state_handles = self.failed_state_handles.clone();
        failed_state_handles.sort_by_key(|failure| failure.handle);
        failed_state_handles.dedup_by_key(|failure| failure.handle);
        let mut unsafe_gaps = self.unsafe_gaps.clone();
        unsafe_gaps.sort_unstable();
        unsafe_gaps.dedup();
        let mut unresolved_object_edges = self.unresolved_object_edges.clone();
        unresolved_object_edges.sort_unstable();
        unresolved_object_edges.dedup();

        MarkOnlyReport {
            runtime_id: self.runtime_id,
            total_objects: self.total_objects,
            marked_objects: gc.marked_object_count(),
            object_trace_steps: self.object_trace_steps,
            traced_states,
            failed_state_handles,
            collector_roots_seeded: self.collector_roots_seeded,
            collector_roots_rejected: self.collector_roots_rejected,
            rejected_child_edges: gc.rejected_mark_edge_count(),
            root_edges: self
                .root_counts
                .iter()
                .map(|(kind, edges)| RootEdgeCount {
                    kind: *kind,
                    edges: *edges,
                })
                .collect(),
            unsafe_gaps,
            unresolved_object_edges,
            pending_objects: gc.pending_mark_count(),
        }
    }
}

/// Canonical Runtime root traversal while the scheduler has released every
/// StateArena turn borrow. The caller owns the stop-the-world invariant.
pub(super) fn trace_roots_mark_only_at_safe_point(
    roots: RuntimeRootSet<'_>,
    state_arena: &mut StateArena,
    native_activations: &mut NativeActivationStack,
    gc: &mut GarbageCollector,
) -> MarkOnlyReport {
    debug_assert!(state_arena.turn_borrow.is_none());
    let total_objects = gc.object_count();
    let seed_report = gc.begin_mark_only();

    let mut root_counts = BTreeMap::new();
    increment(
        &mut root_counts,
        RuntimeRootKind::CollectorExplicitRoot,
        seed_report
            .seeded
            .saturating_sub(seed_report.temporary_seeded)
            .saturating_sub(seed_report.pending_finalizers_seeded),
    );
    increment(
        &mut root_counts,
        RuntimeRootKind::TemporaryProtectedRoots,
        seed_report.temporary_seeded,
    );
    increment(
        &mut root_counts,
        RuntimeRootKind::PendingFinalizers,
        seed_report.pending_finalizers_seeded,
    );
    let fixed_strings_seeded = roots
        .fixed_strings
        .iter()
        .copied()
        .filter(|string| gc.mark_registered(*string))
        .count();
    increment(
        &mut root_counts,
        RuntimeRootKind::FixedStrings,
        fixed_strings_seeded,
    );
    increment(
        &mut root_counts,
        RuntimeRootKind::CoroutineActivationBuffer,
        native_activations.frames.len()
            + native_activations.upvalue_transfers.len()
            + native_activations.gc_frames.len(),
    );
    native_activations.seed_roots(gc);
    let mut unresolved_object_edges = Vec::new();
    mark_runtime_table(
        gc,
        roots.global_root,
        RuntimeRootKind::GlobalTable,
        &mut root_counts,
        &mut unresolved_object_edges,
    );
    mark_runtime_table(
        gc,
        roots.registry_root,
        RuntimeRootKind::Registry,
        &mut root_counts,
        &mut unresolved_object_edges,
    );

    let temporary_state_roots = state_arena.temporary_state_roots();
    increment(
        &mut root_counts,
        RuntimeRootKind::TemporaryStateRoots,
        temporary_state_roots.len(),
    );
    let mut state_queue = VecDeque::from([roots.main_handle]);
    state_queue.extend(temporary_state_roots);
    increment(&mut root_counts, RuntimeRootKind::MainStateEntry, 1);
    let mut attempted_states = HashSet::new();
    let mut traced_states = Vec::new();
    let mut failed_state_handles = Vec::new();
    let mut unsafe_gaps = Vec::new();
    let mut object_trace_steps = 0;

    while !state_queue.is_empty() || gc.pending_mark_count() != 0 {
        while let Some(handle) = state_queue.pop_front() {
            if !attempted_states.insert(handle) {
                continue;
            }

            let state_pointer = match state_arena.resolve_for_trace(handle) {
                Ok(state) => state,
                Err(error) => {
                    failed_state_handles.push(StateTraceFailure { handle, error });
                    continue;
                }
            };

            // SAFETY: StateArena validated runtime, slot, generation,
            // occupancy and borrow state (U-04). The Runtime scheduler has
            // released the current turn borrow at this stop-the-world safe
            // point (U-03). The reference exists only while copying a root
            // snapshot and is dropped before the collector is mutated.
            let snapshot = unsafe {
                snapshot_state(state_pointer.as_ref(), handle, handle == roots.main_handle)
            };
            traced_states.push(handle);
            unsafe_gaps.extend(snapshot.gaps);
            mark_snapshot(
                gc,
                handle,
                snapshot.edges,
                &mut root_counts,
                &mut unresolved_object_edges,
            );
        }

        let Some(step) = gc.propagate_one_marked_object() else {
            continue;
        };
        object_trace_steps += 1;
        if step.traced_thread_caller {
            increment(&mut root_counts, RuntimeRootKind::ThreadCallerChain, 1);
        }
        if let Some(handle) = step.thread_state_handle {
            increment(&mut root_counts, RuntimeRootKind::CoroutineStack, 1);
            state_queue.push_back(handle);
        }
        if let Some(handle) = step.upvalue_state_handle {
            increment(&mut root_counts, RuntimeRootKind::OpenUpvalues, 1);
            state_queue.push_back(handle);
        }
    }

    traced_states.sort_unstable();
    failed_state_handles.sort_by_key(|failure| failure.handle);
    unsafe_gaps.sort_unstable();
    unresolved_object_edges.sort_unstable();

    let marked_objects = gc.marked_object_count();
    debug_assert_eq!(gc.object_count(), total_objects);
    MarkOnlyReport {
        runtime_id: roots.runtime_id,
        total_objects,
        marked_objects,
        object_trace_steps,
        traced_states,
        failed_state_handles,
        collector_roots_seeded: seed_report.seeded,
        collector_roots_rejected: seed_report.rejected,
        rejected_child_edges: gc.rejected_mark_edge_count(),
        root_edges: root_counts
            .into_iter()
            .map(|(kind, edges)| RootEdgeCount { kind, edges })
            .collect(),
        unsafe_gaps,
        unresolved_object_edges,
        pending_objects: gc.pending_mark_count(),
    }
}

fn snapshot_state(state: &LuaState, handle: StateHandle, is_main: bool) -> StateSnapshot {
    let mut snapshot = StateSnapshot::default();
    snapshot.push_table(RuntimeRootKind::GlobalTable, state.global_table);
    snapshot.push_table(RuntimeRootKind::GlobalEnvironments, state.thread_env);
    snapshot.push_table(RuntimeRootKind::GlobalEnvironments, state.chunk_env);
    snapshot.push_table(RuntimeRootKind::Registry, state.registry);
    snapshot.push_table(RuntimeRootKind::PrimitiveMetatables, state.nil_metatable);
    snapshot.push_table(
        RuntimeRootKind::PrimitiveMetatables,
        state.boolean_metatable,
    );
    snapshot.push_table(RuntimeRootKind::PrimitiveMetatables, state.number_metatable);

    if let Some(thread) = state.current_thread {
        snapshot.push_value(RuntimeRootKind::RunningThread, Value::Thread(thread));
    }
    if let Some(hook) = &state.debug_hook {
        snapshot.push_value(RuntimeRootKind::DebugHook, hook.clone());
    }
    if let Some(proto) = state.debug_hook_skip_proto {
        snapshot.push_proto(RuntimeRootKind::DebugProto, proto);
    }
    for value in &state.yielded_values {
        snapshot.push_value(RuntimeRootKind::YieldedValues, value.clone());
    }
    if let Some(error) = &state.last_error {
        snapshot.push_value(RuntimeRootKind::LastError, error.clone());
    }

    let stack_size = state.stack.size();
    if state.top > stack_size {
        snapshot.gaps.push(UnsafeTraceGap {
            state: handle,
            kind: UnsafeTraceGapKind::StackWindowOutOfBounds {
                top: state.top,
                available: stack_size,
            },
        });
    }
    let stack_kind = if is_main {
        RuntimeRootKind::MainStack
    } else {
        RuntimeRootKind::CoroutineStack
    };
    for index in 0..state.top.min(stack_size) {
        if let Some(value) = state.stack.at(index) {
            snapshot.push_value(stack_kind, value.clone());
        }
    }

    let active_frames = match state.current_ci.checked_add(1) {
        Some(requested) if requested <= state.call_stack.len() => requested,
        _ => {
            snapshot.gaps.push(UnsafeTraceGap {
                state: handle,
                kind: UnsafeTraceGapKind::ActiveCallStackOutOfBounds {
                    current_ci: state.current_ci,
                    available: state.call_stack.len(),
                },
            });
            state.call_stack.len()
        }
    };
    for (frame, call_info) in state.call_stack.iter().take(active_frames).enumerate() {
        let idle_placeholder = frame == 0
            && active_frames == 1
            && state.top == 0
            && stack_size == 0
            && call_info.func == 0
            && call_info.base == 0
            && call_info.top == 0
            && call_info.savedpc.is_none()
            && call_info.proto.is_none()
            && call_info.varargs.is_empty();
        if idle_placeholder {
            // A freshly created or fully unwound LuaState retains one empty
            // CallInfo sentinel. It owns no function stack edge.
        } else if call_info.func < stack_size
            && let Some(value) = state.stack.at(call_info.func)
        {
            snapshot.push_value(RuntimeRootKind::CallFunction, value.clone());
        } else {
            snapshot.gaps.push(UnsafeTraceGap {
                state: handle,
                kind: UnsafeTraceGapKind::CallFunctionOutOfBounds {
                    frame,
                    function_index: call_info.func,
                },
            });
        }
        for value in &call_info.varargs {
            snapshot.push_value(RuntimeRootKind::CallVarargs, value.clone());
        }
        if let Some(proto) = call_info.proto {
            snapshot.push_proto(RuntimeRootKind::ActiveProto, proto);
        }
    }

    for &open_upvalue in &state.open_upvalues {
        snapshot.edges.push(RootEdge {
            kind: RuntimeRootKind::OpenUpvalues,
            object: RootObject::Upvalue(open_upvalue),
        });
    }
    snapshot
}

fn mark_snapshot(
    gc: &mut lua_core::gc::collector::GarbageCollector,
    state: StateHandle,
    edges: Vec<RootEdge>,
    root_counts: &mut BTreeMap<RuntimeRootKind, usize>,
    unresolved: &mut Vec<UnresolvedObjectEdge>,
) {
    for edge in edges {
        increment(root_counts, edge.kind, 1);
        let resolved = match edge.object {
            RootObject::Value(value) => gc.mark_registered_value(&value),
            RootObject::Proto(value) => gc.mark_registered(value),
            RootObject::Upvalue(value) => gc.mark_registered(value),
        };
        if !resolved {
            unresolved.push(UnresolvedObjectEdge {
                state: Some(state),
                kind: edge.kind,
            });
        }
    }
}

fn mark_runtime_table(
    gc: &mut lua_core::gc::collector::GarbageCollector,
    table: Option<GcRef<Table>>,
    kind: RuntimeRootKind,
    root_counts: &mut BTreeMap<RuntimeRootKind, usize>,
    unresolved: &mut Vec<UnresolvedObjectEdge>,
) {
    let Some(table) = table else {
        return;
    };
    increment(root_counts, kind, 1);
    if !gc.mark_registered(table) {
        unresolved.push(UnresolvedObjectEdge { state: None, kind });
    }
}

fn increment(
    root_counts: &mut BTreeMap<RuntimeRootKind, usize>,
    kind: RuntimeRootKind,
    count: usize,
) {
    *root_counts.entry(kind).or_default() += count;
}

#[cfg(test)]
mod tests {
    use lua_core::function::Function;
    use lua_core::gc::collector::GarbageCollector;
    use lua_core::gc::gc_ref::GcRef;
    use lua_core::gc::header::GcObjectHeader;
    use lua_core::gc_string::GcString;
    use lua_core::proto::Proto;
    use lua_core::table::Table;
    use lua_core::thread::Thread;
    use lua_core::value::Value;

    use super::*;

    struct StateRootFixtures {
        explicit: GcRef<Table>,
        thread_env: GcRef<Table>,
        chunk_env: GcRef<Table>,
        nil_metatable: GcRef<Table>,
        boolean_metatable: GcRef<Table>,
        number_metatable: GcRef<Table>,
        running_thread: GcRef<Thread>,
        active_stack: GcRef<Table>,
        active_function: GcRef<Function>,
        active_proto: GcRef<Proto>,
        vararg: GcRef<Table>,
        open_value: GcRef<Table>,
        open_upvalue: GcRef<Upvalue>,
        debug_hook: GcRef<Table>,
        yielded: GcRef<Table>,
        last_error: GcRef<GcString>,
        retired_stack: GcRef<Table>,
        inactive_vararg: GcRef<Table>,
    }

    #[test]
    fn mark_only_traces_runtime_state_roots_and_excludes_retired_values() {
        let mut runtime = Runtime::new();
        let global = runtime.global_root().expect("global root exists");
        let registry = runtime.registry_root().expect("registry root exists");

        let fixtures = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, gc, string_pool) = parts.split_mut();
            let explicit = gc.create_root(Table::new());
            let thread_env = gc.create(Table::new());
            let chunk_env = gc.create(Table::new());
            let nil_metatable = gc.create(Table::new());
            let boolean_metatable = gc.create(Table::new());
            let number_metatable = gc.create(Table::new());
            let running_thread = gc.create(Thread::new());
            let active_stack = gc.create(Table::new());
            let active_proto = gc.create(Proto::new());
            let active_function = gc.create(Function::new_lua(active_proto));
            let vararg = gc.create(Table::new());
            let open_value = gc.create(Table::new());
            let debug_hook = gc.create(Table::new());
            let yielded = gc.create(Table::new());
            let last_error = string_pool.intern_bytes(gc, b"last error");
            let retired_stack = gc.create(Table::new());
            let inactive_vararg = gc.create(Table::new());

            state.global_table = Some(global);
            state.thread_env = Some(thread_env);
            state.chunk_env = Some(chunk_env);
            state.registry = Some(registry);
            state.nil_metatable = Some(nil_metatable);
            state.boolean_metatable = Some(boolean_metatable);
            state.number_metatable = Some(number_metatable);
            state.current_thread = Some(running_thread);
            state.debug_hook = Some(Value::Table(debug_hook));
            state.yielded_values.push(Value::Table(yielded));
            state.last_error = Some(Value::String(last_error));

            state.push_value(Value::Function(active_function));
            state.push_value(Value::Table(active_stack));
            let open_index = state.top;
            state.push_value(Value::Table(open_value));
            let open_upvalue = state
                .find_or_create_upvalue(open_index, gc)
                .expect("runtime-owned state publishes open Upvalue");
            state.push_value(Value::Table(retired_stack));
            assert_eq!(state.pop(), Some(Value::Table(retired_stack)));

            state.current_call_info_mut().func = 0;
            state.current_call_info_mut().proto = Some(active_proto);
            state.current_call_info_mut().varargs = vec![Value::Table(vararg)];
            state.debug_hook_skip_proto = Some(active_proto);
            state.push_call_info().varargs = vec![Value::Table(inactive_vararg)];
            state.pop_call_info();

            StateRootFixtures {
                explicit,
                thread_env,
                chunk_env,
                nil_metatable,
                boolean_metatable,
                number_metatable,
                running_thread,
                active_stack,
                active_function,
                active_proto,
                vararg,
                open_value,
                open_upvalue,
                debug_hook,
                yielded,
                last_error,
                retired_stack,
                inactive_vararg,
            }
        };

        let object_count = runtime.heap.as_ref().get_ref().heap.object_count();
        let report = runtime
            .trace_roots_mark_only()
            .expect("mark-only traversal succeeds");

        assert_eq!(report.total_objects, object_count);
        assert_eq!(
            runtime.heap.as_ref().get_ref().heap.object_count(),
            object_count
        );
        assert_eq!(report.pending_objects, 0);
        assert!(report.failed_state_handles.is_empty());
        assert!(report.unresolved_object_edges.is_empty());
        for kind in [
            RuntimeRootKind::CollectorExplicitRoot,
            RuntimeRootKind::GlobalTable,
            RuntimeRootKind::GlobalEnvironments,
            RuntimeRootKind::Registry,
            RuntimeRootKind::PrimitiveMetatables,
            RuntimeRootKind::MainStateEntry,
            RuntimeRootKind::RunningThread,
            RuntimeRootKind::MainStack,
            RuntimeRootKind::CallFunction,
            RuntimeRootKind::ActiveProto,
            RuntimeRootKind::DebugProto,
            RuntimeRootKind::CallVarargs,
            RuntimeRootKind::OpenUpvalues,
            RuntimeRootKind::DebugHook,
            RuntimeRootKind::YieldedValues,
            RuntimeRootKind::LastError,
        ] {
            assert!(
                report.root_edge_count(kind) > 0,
                "{} was not traced",
                kind.inventory_name()
            );
        }
        assert_eq!(
            report.root_edge_count(RuntimeRootKind::ThreadCallerChain),
            0
        );

        assert_marked(global);
        assert_marked(registry);
        assert_marked(fixtures.explicit);
        assert_marked(fixtures.thread_env);
        assert_marked(fixtures.chunk_env);
        assert_marked(fixtures.nil_metatable);
        assert_marked(fixtures.boolean_metatable);
        assert_marked(fixtures.number_metatable);
        assert_marked(fixtures.running_thread);
        assert_marked(fixtures.active_stack);
        assert_marked(fixtures.active_function);
        assert_marked(fixtures.active_proto);
        assert_marked(fixtures.vararg);
        assert_marked(fixtures.open_value);
        assert_marked(fixtures.open_upvalue);
        assert_marked(fixtures.debug_hook);
        assert_marked(fixtures.yielded);
        assert_marked(fixtures.last_error);
        assert_white(fixtures.retired_stack);
        assert_white(fixtures.inactive_vararg);
        assert!(report.unsafe_gaps.is_empty());

        {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, gc, _) = parts.split_mut();
            gc.remove_root(global);
            gc.remove_root(registry);
            gc.remove_root(fixtures.explicit);
            state.global_table = None;
            state.thread_env = None;
            state.chunk_env = None;
            state.registry = None;
            state.nil_metatable = None;
            state.boolean_metatable = None;
            state.number_metatable = None;
            state.current_thread = None;
            state.debug_hook = None;
            state.debug_hook_skip_proto = None;
            state.yielded_values.clear();
            state.last_error = None;
            state.open_upvalues.clear();
            state.current_call_info_mut().proto = None;
            state.current_call_info_mut().varargs.clear();
            state.set_top(0);
        }
        runtime.global_root = None;
        runtime.registry_root = None;

        let absent = runtime
            .trace_roots_mark_only()
            .expect("cleared roots produce another mark-only report");
        let fixed_string_count = lua_core::metatable::METAMETHOD_NAMES.len()
            + crate::runtime::FIXED_RUNTIME_STRING_BYTES.len();
        assert_eq!(absent.marked_objects, fixed_string_count);
        assert_eq!(absent.collector_roots_seeded, 0);
        assert_eq!(
            absent.root_edge_count(RuntimeRootKind::FixedStrings),
            fixed_string_count
        );
        assert_eq!(absent.root_edge_count(RuntimeRootKind::MainStateEntry), 1);
        for kind in [
            RuntimeRootKind::CollectorExplicitRoot,
            RuntimeRootKind::GlobalTable,
            RuntimeRootKind::GlobalEnvironments,
            RuntimeRootKind::Registry,
            RuntimeRootKind::PrimitiveMetatables,
            RuntimeRootKind::RunningThread,
            RuntimeRootKind::MainStack,
            RuntimeRootKind::CoroutineStack,
            RuntimeRootKind::CallFunction,
            RuntimeRootKind::ActiveProto,
            RuntimeRootKind::DebugProto,
            RuntimeRootKind::CallVarargs,
            RuntimeRootKind::OpenUpvalues,
            RuntimeRootKind::ThreadCallerChain,
            RuntimeRootKind::DebugHook,
            RuntimeRootKind::YieldedValues,
            RuntimeRootKind::LastError,
            RuntimeRootKind::TemporaryStateRoots,
        ] {
            assert_eq!(
                absent.root_edge_count(kind),
                0,
                "{} unexpectedly remained a root",
                kind.inventory_name()
            );
        }
        assert_white(global);
        assert_white(registry);
        assert_white(fixtures.explicit);
        assert_white(fixtures.thread_env);
        assert_white(fixtures.chunk_env);
        assert_white(fixtures.nil_metatable);
        assert_white(fixtures.boolean_metatable);
        assert_white(fixtures.number_metatable);
        assert_white(fixtures.running_thread);
        assert_white(fixtures.active_stack);
        assert_white(fixtures.active_function);
        assert_white(fixtures.active_proto);
        assert_white(fixtures.vararg);
        assert_white(fixtures.open_value);
        assert_white(fixtures.open_upvalue);
        assert_white(fixtures.debug_hook);
        assert_white(fixtures.yielded);
        assert_white(fixtures.last_error);
        assert_white(fixtures.retired_stack);
        assert_white(fixtures.inactive_vararg);
    }

    #[test]
    fn pending_state_is_traced_before_any_thread_edge_exists() {
        let mut runtime = Runtime::new();
        let main_handle = runtime.main_state_handle.expect("main state handle");

        let (pending_handle, temporary_root_id, payload) = {
            // SAFETY: RuntimeStorage is pinned and no execution guard is live.
            let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
            let payload = heap.heap.collector_mut().create(Table::new());
            let mut pending = LuaState::new();
            pending.push_value(Value::Table(payload));
            let (handle, root_id) = heap
                .state_arena
                .insert_pending_owned(Box::new(pending))
                .expect("detached state enters the pending root set");
            (handle, root_id, payload)
        };

        let report = runtime
            .trace_roots_mark_only()
            .expect("temporary state root is a traceable handle");

        assert_eq!(
            report.traced_states,
            sorted_handles([main_handle, pending_handle])
        );
        assert_eq!(
            report.root_edge_count(RuntimeRootKind::TemporaryStateRoots),
            1
        );
        assert!(report.failed_state_handles.is_empty());
        assert!(report.unresolved_object_edges.is_empty());
        assert_marked(payload);

        // SAFETY: the exact pending root remains live in this pinned arena and
        // no state or execution borrow is active.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .rollback_pending(pending_handle, temporary_root_id)
            .expect("exact pending root rolls back");
        assert_eq!(heap.state_arena.temporary_state_root_count(), 0);
        assert_eq!(heap.state_arena.live_owned_state_count(), 0);
    }

    #[test]
    fn active_call_info_proto_is_rooted_without_a_function_slot() {
        let mut runtime = Runtime::new();
        let active_proto = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, gc, _) = parts.split_mut();
            let active_proto = gc.create(Proto::new());
            state.current_call_info_mut().proto = Some(active_proto);
            active_proto
        };

        let report = runtime
            .trace_roots_mark_only()
            .expect("managed active Proto root traces");

        assert_eq!(report.root_edge_count(RuntimeRootKind::ActiveProto), 1);
        assert!(report.unresolved_object_edges.is_empty());
        assert_marked(active_proto);
    }

    #[test]
    fn mark_only_reaches_two_queue_fixed_point_without_rooting_arena_membership() {
        let mut runtime = Runtime::new();
        let main_handle = runtime.main_state_handle.expect("main state handle");

        let (thread_a, thread_b, payload, unreachable, state_a, state_b, unrooted_state) = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (main, gc, _) = parts.split_mut();
            let payload = gc.create(Table::new());
            let unreachable = gc.create(Table::new());

            let mut state_b_value = LuaState::new();
            state_b_value.push_value(Value::Table(payload));
            let state_b = main
                .insert_coroutine_state(state_b_value)
                .expect("state B inserted");

            let state_a = main
                .insert_coroutine_state(LuaState::new())
                .expect("state A inserted");
            let mut thread_a_value = Thread::new();
            thread_a_value.set_state_handle(state_a);
            let thread_a = gc.create(thread_a_value);

            let mut thread_b_value = Thread::new();
            thread_b_value.set_state_handle(state_b);
            thread_b_value.set_caller(Some(thread_a));
            let thread_b = gc.create(thread_b_value);
            main.with_resolved_state_mut(state_a, |state| {
                state.push_value(Value::Thread(thread_b));
            })
            .expect("state A resolves");
            main.push_value(Value::Thread(thread_a));

            let mut unrooted_state_value = LuaState::new();
            unrooted_state_value.push_value(Value::Table(unreachable));
            let unrooted_state = main
                .insert_coroutine_state(unrooted_state_value)
                .expect("unreachable state inserted");

            (
                thread_a,
                thread_b,
                payload,
                unreachable,
                state_a,
                state_b,
                unrooted_state,
            )
        };

        let report = runtime
            .trace_roots_mark_only()
            .expect("mark-only traversal succeeds");

        assert_eq!(
            report.traced_states,
            sorted_handles([main_handle, state_a, state_b])
        );
        assert!(!report.traced_states.contains(&unrooted_state));
        assert!(report.failed_state_handles.is_empty());
        assert!(report.root_edge_count(RuntimeRootKind::CoroutineStack) >= 3);
        assert_eq!(
            report.root_edge_count(RuntimeRootKind::ThreadCallerChain),
            1
        );
        assert_marked(thread_a);
        assert_marked(thread_b);
        assert_marked(payload);
        assert_white(unreachable);
    }

    #[test]
    fn reachable_open_upvalue_enqueues_its_owner_state_without_a_thread_edge() {
        let mut runtime = Runtime::new();
        let main_handle = runtime.main_state_handle.expect("main state handle");

        let (owner_handle, closure, open_upvalue, open_value) = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (main, gc, _) = parts.split_mut();
            let owner_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("owner state inserted");
            let open_value = gc.create(Table::new());
            let open_upvalue = main
                .with_resolved_state_mut(owner_handle, |owner| {
                    owner.push_value(Value::Table(open_value));
                    owner
                        .find_or_create_upvalue(0, gc)
                        .expect("owner publishes open Upvalue")
                })
                .expect("owner state resolves");

            let proto = gc.create(Proto::new());
            let mut function = Function::new_lua(proto);
            function.add_upvalue(open_upvalue);
            let closure = gc.create(function);
            main.push_value(Value::Function(closure));

            (owner_handle, closure, open_upvalue, open_value)
        };

        let report = runtime
            .trace_roots_mark_only()
            .expect("open Upvalue owner traversal succeeds");

        assert_eq!(
            report.traced_states,
            sorted_handles([main_handle, owner_handle])
        );
        assert_eq!(
            report.root_edge_count(RuntimeRootKind::ThreadCallerChain),
            0
        );
        assert!(report.root_edge_count(RuntimeRootKind::OpenUpvalues) >= 2);
        assert!(report.failed_state_handles.is_empty());
        assert!(report.unsafe_gaps.is_empty());
        assert_marked(closure);
        assert_marked(open_upvalue);
        assert_marked(open_value);
    }

    #[test]
    fn mark_only_reports_stale_and_foreign_state_handles_without_dereference() {
        let mut runtime = Runtime::new();
        let stale = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("stale candidate is inserted")
        };
        // SAFETY: RuntimeStorage is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .remove_owned(stale)
            .expect("stale candidate is removed");

        let foreign_runtime = Runtime::new();
        let foreign = foreign_runtime
            .main_state_handle
            .expect("foreign main state handle");

        {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, gc, _) = parts.split_mut();
            let mut stale_thread = Thread::new();
            stale_thread.set_state_handle(stale);
            let stale_thread = gc.create(stale_thread);
            let mut foreign_thread = Thread::new();
            foreign_thread.set_state_handle(foreign);
            let foreign_thread = gc.create(foreign_thread);
            state.push_value(Value::Thread(stale_thread));
            state.push_value(Value::Thread(foreign_thread));
        }

        let object_count = runtime.heap.as_ref().get_ref().heap.object_count();
        let report = runtime
            .trace_roots_mark_only()
            .expect("invalid edges are diagnostics, not traversal failure");

        assert_eq!(
            runtime.heap.as_ref().get_ref().heap.object_count(),
            object_count
        );
        assert_eq!(report.failed_state_handles.len(), 2);
        assert!(report.failed_state_handles.iter().any(|failure| {
            failure.handle == stale
                && matches!(failure.error, StateResolveError::StaleGeneration { .. })
        }));
        assert!(report.failed_state_handles.iter().any(|failure| {
            failure.handle == foreign
                && matches!(failure.error, StateResolveError::ForeignRuntime { .. })
        }));
    }

    #[test]
    fn mark_only_rejects_cross_collector_child_before_dereference() {
        let mut runtime = Runtime::new();
        let mut foreign_gc = GarbageCollector::new();
        let foreign = foreign_gc.create(Table::new());

        let local_root = {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (_, gc, _) = parts.split_mut();
            let local_root = gc.create_root(Table::new());
            // SAFETY: `local_root` remains registered in the Runtime
            // collector and mark-only tracing does not relocate objects.
            unsafe {
                (&mut *local_root.as_ptr().cast_mut())
                    .set(&Value::Number(1.0), &Value::Table(foreign));
            }
            local_root
        };

        let report = runtime
            .trace_roots_mark_only()
            .expect("foreign child is reported instead of dereferenced");

        assert_eq!(report.rejected_child_edges, 1);
        assert_marked(local_root);
        assert_white(foreign);
        assert_eq!(foreign_gc.marked_object_count(), 0);
    }

    #[test]
    fn mark_only_rejects_active_execution_state() {
        let mut runtime = Runtime::new();
        runtime.active_executions = 1;
        assert!(matches!(
            runtime.trace_roots_mark_only(),
            Err(RuntimeAccessError::ActiveExecutions { count: 1, .. })
        ));
        runtime.active_executions = 0;
    }

    fn sorted_handles<const N: usize>(handles: [StateHandle; N]) -> Vec<StateHandle> {
        let mut handles = handles.to_vec();
        handles.sort_unstable();
        handles
    }

    fn assert_marked<T>(object: GcRef<T>) {
        let header = object.as_ptr() as *const GcObjectHeader;
        // SAFETY: tests call this only for objects still registered in the
        // Runtime collector; mark-only tracing never destroys or relocates.
        assert!(!unsafe { (*header).is_white() });
    }

    fn assert_white<T>(object: GcRef<T>) {
        let header = object.as_ptr() as *const GcObjectHeader;
        // SAFETY: same lifetime argument as `assert_marked`.
        assert!(unsafe { (*header).is_white() });
    }
}

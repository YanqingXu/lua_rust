//! Runtime-owned, mark-only root traversal.
//!
//! This is intentionally a live-set diagnostic, not a collector cycle. It
//! mutates only mark colors/work queues and never calls destructive sweep,
//! collection, finalization, or shutdown APIs.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::pin::Pin;

use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::state_handle::{RuntimeId, StateHandle};
use lua_core::table::Table;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use crate::state::LuaState;

use super::{Runtime, RuntimeAccessError, RuntimePhase, StateResolveError};

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
    /// An open Upvalue still stores a raw owner-Stack pointer.
    OpenUpvalueOwnerStackRaw,
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

        // SAFETY: RuntimeHeap is pinned, Runtime is exclusively borrowed, and
        // the active-execution check establishes U-03 for immutable state
        // snapshots alongside mutable collector mark state.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let total_objects = heap.gc.object_count();
        let seed_report = heap.gc.begin_mark_only();

        let mut root_counts = BTreeMap::new();
        increment(
            &mut root_counts,
            RuntimeRootKind::CollectorExplicitRoot,
            seed_report.seeded,
        );
        let mut unresolved_object_edges = Vec::new();
        mark_runtime_table(
            &mut heap.gc,
            global_root,
            RuntimeRootKind::GlobalTable,
            &mut root_counts,
            &mut unresolved_object_edges,
        );
        mark_runtime_table(
            &mut heap.gc,
            registry_root,
            RuntimeRootKind::Registry,
            &mut root_counts,
            &mut unresolved_object_edges,
        );

        let mut state_queue = VecDeque::from([main_handle]);
        increment(&mut root_counts, RuntimeRootKind::MainStateEntry, 1);
        let mut attempted_states = HashSet::new();
        let mut traced_states = Vec::new();
        let mut failed_state_handles = Vec::new();
        let mut unsafe_gaps = Vec::new();
        let mut object_trace_steps = 0;

        while !state_queue.is_empty() || heap.gc.pending_mark_count() != 0 {
            while let Some(handle) = state_queue.pop_front() {
                if !attempted_states.insert(handle) {
                    continue;
                }

                let state_pointer = match heap.state_arena.resolve_for_trace(handle) {
                    Ok(state) => state,
                    Err(error) => {
                        failed_state_handles.push(StateTraceFailure { handle, error });
                        continue;
                    }
                };

                // SAFETY: StateArena validated runtime, slot, generation,
                // occupancy and borrow state (U-04). Runtime is exclusively
                // borrowed with zero active executions (U-03). The reference
                // exists only while copying a root snapshot and is dropped
                // before the collector is mutated.
                let snapshot = unsafe {
                    snapshot_state(state_pointer.as_ref(), handle, handle == main_handle)
                };
                traced_states.push(handle);
                unsafe_gaps.extend(snapshot.gaps);
                mark_snapshot(
                    &mut heap.gc,
                    handle,
                    snapshot.edges,
                    &mut root_counts,
                    &mut unresolved_object_edges,
                );
            }

            let Some(step) = heap.gc.propagate_one_marked_object() else {
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
        }

        traced_states.sort_unstable();
        failed_state_handles.sort_by_key(|failure| failure.handle);
        unsafe_gaps.sort_unstable();
        unresolved_object_edges.sort_unstable();

        let marked_objects = heap.gc.marked_object_count();
        debug_assert_eq!(heap.gc.object_count(), total_objects);
        Ok(MarkOnlyReport {
            runtime_id: self.id,
            total_objects,
            marked_objects,
            object_trace_steps,
            traced_states,
            failed_state_handles,
            collector_roots_seeded: seed_report.seeded,
            collector_roots_rejected: seed_report.rejected,
            rejected_child_edges: heap.gc.rejected_mark_edge_count(),
            root_edges: root_counts
                .into_iter()
                .map(|(kind, edges)| RootEdgeCount { kind, edges })
                .collect(),
            unsafe_gaps,
            unresolved_object_edges,
            pending_objects: heap.gc.pending_mark_count(),
        })
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
        if call_info.func < stack_size
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

    if let Some(open_upvalue) = state.open_upvalues {
        snapshot.edges.push(RootEdge {
            kind: RuntimeRootKind::OpenUpvalues,
            object: RootObject::Upvalue(open_upvalue),
        });
        snapshot.gaps.push(UnsafeTraceGap {
            state: handle,
            kind: UnsafeTraceGapKind::OpenUpvalueOwnerStackRaw,
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
            let (state, gc, _) = parts.split_mut();
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
            let last_error = gc.create(GcString::from_bytes(b"last error"));
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
            let open_upvalue = state.find_or_create_upvalue(open_index, gc);
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

        let object_count = runtime.heap.as_ref().get_ref().gc.object_count();
        let report = runtime
            .trace_roots_mark_only()
            .expect("mark-only traversal succeeds");

        assert_eq!(report.total_objects, object_count);
        assert_eq!(
            runtime.heap.as_ref().get_ref().gc.object_count(),
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
        assert!(
            report
                .unsafe_gaps
                .iter()
                .any(|gap| { gap.kind == UnsafeTraceGapKind::OpenUpvalueOwnerStackRaw })
        );

        {
            let mut parts = runtime.parts_mut().expect("runtime parts");
            let (state, gc, _) = parts.split_mut();
            gc.remove_root(global);
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
            state.open_upvalues = None;
            state.current_call_info_mut().proto = None;
            state.current_call_info_mut().varargs.clear();
            state.set_top(0);
        }
        runtime.global_root = None;
        runtime.registry_root = None;

        let absent = runtime
            .trace_roots_mark_only()
            .expect("cleared roots produce another mark-only report");
        assert_eq!(absent.marked_objects, 0);
        assert_eq!(absent.collector_roots_seeded, 0);
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
    fn mark_only_reports_stale_and_foreign_state_handles_without_dereference() {
        let mut runtime = Runtime::new();
        let main_handle = runtime.main_state_handle.expect("main state handle");
        let stale = StateHandle::new(
            runtime.id(),
            main_handle.slot(),
            main_handle.generation().wrapping_add(1),
        );
        let foreign = StateHandle::new(RuntimeId::new(u64::MAX), 0, 1);

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

        let object_count = runtime.heap.as_ref().get_ref().gc.object_count();
        let report = runtime
            .trace_roots_mark_only()
            .expect("invalid edges are diagnostics, not traversal failure");

        assert_eq!(
            runtime.heap.as_ref().get_ref().gc.object_count(),
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

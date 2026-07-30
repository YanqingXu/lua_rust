//! Runtime-owned stop-the-world full collection.
//!
//! Lua-visible `collectgarbage` requests enter here only after the scheduler
//! releases its current state turn. The atomic phase consumes the canonical
//! Runtime root tracer, prepares finalizers, propagates resurrected graphs,
//! reconciles weak tables, closes unreachable coroutine states before object
//! destruction, and then sweeps. Explicit incremental collection and
//! allocation-triggered cycles share these Runtime safe points.

use lua_core::gc::collector::GarbageCollector;
use lua_core::state_handle::{RuntimeId, StateHandle};
use lua_core::string_pool::StringPool;
use std::collections::HashSet;
use thiserror::Error;

use super::root_trace::{RuntimeRootSet, trace_roots_mark_only_at_safe_point};
use super::{
    MarkOnlyReport, NativeActivationStack, Runtime, RuntimeAccessError, StateArena,
    StateResolveError,
};

/// Result of one Runtime-owned stop-the-world full collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeFullCollectionReport {
    pub(crate) runtime_id: RuntimeId,
    pub(crate) mark: MarkOnlyReport,
    pub(crate) objects_before: usize,
    pub(crate) objects_after: usize,
    pub(crate) collected_objects: usize,
    pub(crate) accounted_bytes_before: usize,
    pub(crate) accounted_bytes_after: usize,
    pub(crate) reclaimed_accounted_bytes: usize,
    pub(crate) interned_strings_before: usize,
    pub(crate) interned_strings_after: usize,
    pub(crate) coroutine_states_before: usize,
    pub(crate) coroutine_states_after: usize,
    pub(crate) newly_discovered_finalizers: usize,
    pub(crate) pending_finalizers_after: usize,
    pub(crate) swept_state_handles: Vec<StateHandle>,
    pub(crate) closed_open_upvalues: usize,
    pub(crate) rejected_open_upvalue_edges: usize,
    pub(crate) open_upvalue_owner_mismatches: usize,
    pub(crate) open_upvalue_stack_values_missing: usize,
}

/// Fail-closed reason that prevented an internal destructive collection.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum RuntimeFullCollectionError {
    #[error(transparent)]
    Access(#[from] RuntimeAccessError),
    #[error(
        "canonical root trace is not sweep-safe: failed states={failed_states}, \
         unsafe gaps={unsafe_gaps}, unresolved edges={unresolved_edges}, \
         rejected roots={rejected_roots}, rejected child edges={rejected_child_edges}, \
         pending mark work={pending_mark_work}"
    )]
    UnsafeRootTrace {
        failed_states: usize,
        unsafe_gaps: usize,
        unresolved_edges: usize,
        rejected_roots: usize,
        rejected_child_edges: usize,
        pending_mark_work: usize,
    },
    #[error("coroutine state pre-sweep failed: {source}")]
    StatePrepass {
        #[source]
        source: StateResolveError,
    },
}

impl Runtime {
    /// Run one direct, non-reentrant stop-the-world full collection.
    ///
    /// The Runtime must be running on its owner thread with no active
    /// execution guard. Runtime-native `collectgarbage` requests use
    /// `collect_full_stw_at_safe_point` instead so the activation snapshot
    /// remains part of the canonical root set.
    #[allow(dead_code, reason = "direct collector entry is exercised by VM tests")]
    pub(crate) fn collect_full_stw(
        &mut self,
    ) -> Result<RuntimeFullCollectionReport, RuntimeFullCollectionError> {
        self.check_owner()?;
        if self.phase != super::RuntimePhase::Running {
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
        let main_handle =
            self.main_state_handle
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;
        // SAFETY: RuntimeStorage is pinned and the access checks above prove
        // there is no active state turn.
        let storage = unsafe { std::pin::Pin::get_unchecked_mut(self.heap.as_mut()) };
        storage.incremental_trace = None;
        let (gc, strings) = storage.heap.parts_mut();
        gc.abort_incremental_cycle();
        collect_full_stw_at_safe_point(
            RuntimeRootSet {
                runtime_id: self.id,
                main_handle,
                global_root: self.global_root,
                registry_root: self.registry_root,
                fixed_strings: &self.fixed_strings,
            },
            &mut storage.state_arena,
            &mut storage.native_activations,
            gc,
            strings,
        )
    }
}

/// Run the destructive atomic phase after the Runtime scheduler has released
/// its current StateArena turn borrow.
pub(super) fn collect_full_stw_at_safe_point(
    roots: RuntimeRootSet<'_>,
    state_arena: &mut StateArena,
    native_activations: &mut NativeActivationStack,
    gc: &mut GarbageCollector,
    strings: &mut StringPool,
) -> Result<RuntimeFullCollectionReport, RuntimeFullCollectionError> {
    debug_assert!(state_arena.turn_borrow.is_none());
    let initial_mark =
        trace_roots_mark_only_at_safe_point(roots, state_arena, native_activations, gc);
    ensure_sweep_safe(&initial_mark)?;

    let objects_before = gc.object_count();
    let accounted_bytes_before = gc.total_memory();
    let interned_strings_before = strings.len();
    let coroutine_states_before = state_arena.live_owned_state_count();

    let newly_discovered = gc.prepare_finalizable_userdata().len();

    // Pending finalizers are canonical collector roots, so this second
    // Runtime fixed point both
    // propagates their resurrection graph and discovers any Thread or
    // open-Upvalue StateHandle edges introduced by that graph. It also
    // re-reads weak modes before atomic weak cleanup.
    let mark = if newly_discovered == 0 {
        initial_mark
    } else {
        let finalizer_mark =
            trace_roots_mark_only_at_safe_point(roots, state_arena, native_activations, gc);
        ensure_sweep_safe(&finalizer_mark)?;
        finalizer_mark
    };
    let reachable_states: HashSet<_> = mark.traced_states.iter().copied().collect();

    gc.clear_weak_table_entries();

    let state_report = state_arena
        .sweep_unreachable_owned(&reachable_states, gc)
        .map_err(|source| RuntimeFullCollectionError::StatePrepass { source })?;

    let collected_objects = gc.sweep(strings);
    let objects_after = gc.object_count();
    let accounted_bytes_after = gc.total_memory();
    let interned_strings_after = strings.len();
    let coroutine_states_after = state_arena.live_owned_state_count();
    let pending_finalizers_after = gc.pending_finalizer_count();

    debug_assert_eq!(
        objects_before.saturating_sub(objects_after),
        collected_objects
    );
    debug_assert_eq!(gc.pending_mark_count(), 0);

    Ok(RuntimeFullCollectionReport {
        runtime_id: roots.runtime_id,
        mark,
        objects_before,
        objects_after,
        collected_objects,
        accounted_bytes_before,
        accounted_bytes_after,
        reclaimed_accounted_bytes: accounted_bytes_before.saturating_sub(accounted_bytes_after),
        interned_strings_before,
        interned_strings_after,
        coroutine_states_before,
        coroutine_states_after,
        newly_discovered_finalizers: newly_discovered,
        pending_finalizers_after,
        swept_state_handles: state_report.swept_state_handles,
        closed_open_upvalues: state_report.state_shutdown.closed_open_upvalues,
        rejected_open_upvalue_edges: state_report.state_shutdown.rejected_open_upvalue_edges,
        open_upvalue_owner_mismatches: state_report.state_shutdown.open_upvalue_owner_mismatches,
        open_upvalue_stack_values_missing: state_report
            .state_shutdown
            .open_upvalue_stack_values_missing,
    })
}

pub(super) fn ensure_sweep_safe(mark: &MarkOnlyReport) -> Result<(), RuntimeFullCollectionError> {
    let failed_states = mark.failed_state_handles.len();
    let unsafe_gaps = mark.unsafe_gaps.len();
    let unresolved_edges = mark.unresolved_object_edges.len();
    if failed_states != 0
        || unsafe_gaps != 0
        || unresolved_edges != 0
        || mark.collector_roots_rejected != 0
        || mark.rejected_child_edges != 0
        || mark.pending_objects != 0
    {
        return Err(RuntimeFullCollectionError::UnsafeRootTrace {
            failed_states,
            unsafe_gaps,
            unresolved_edges,
            rejected_roots: mark.collector_roots_rejected,
            rejected_child_edges: mark.rejected_child_edges,
            pending_mark_work: mark.pending_objects,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use lua_core::gc::collector::GarbageCollector;
    use lua_core::table::Table;
    use lua_core::thread::Thread;
    use lua_core::userdata::Userdata;
    use lua_core::value::Value;

    use crate::state::LuaState;

    use super::*;

    struct DropProbe(Rc<Cell<usize>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn full_collection_keeps_reachable_objects_and_reclaims_white_objects_and_bytes() {
        let mut runtime = Runtime::new();
        let (reachable, unreachable) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, _) = parts.split_mut();
            let reachable = collector.create(Table::new());
            let unreachable = collector.create(Table::new());
            state.push_value(Value::Table(reachable));
            (reachable, unreachable)
        };

        let first = runtime
            .collect_full_stw()
            .expect("strong Runtime graph is sweep-safe");
        assert_eq!(first.runtime_id, runtime.id());
        assert_eq!(first.collected_objects, 1);
        assert_eq!(first.objects_before - first.objects_after, 1);
        assert!(first.accounted_bytes_after < first.accounted_bytes_before);
        assert_eq!(
            first.reclaimed_accounted_bytes,
            first.accounted_bytes_before - first.accounted_bytes_after
        );
        assert_eq!(first.interned_strings_before, first.interned_strings_after);
        assert!(
            first.mark.traced_states.contains(
                &runtime
                    .main_state_handle
                    .expect("running Runtime has a main handle")
            )
        );

        {
            let mut parts = runtime.parts_mut().expect("Runtime remains usable");
            let (state, collector, _) = parts.split_mut();
            assert!(collector.contains_registered(reachable));
            assert!(!collector.contains_registered(unreachable));
            state.set_top(0);
        }

        let second = runtime
            .collect_full_stw()
            .expect("a second full cycle resets and reuses mark colors");
        assert_eq!(second.collected_objects, 1);
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(!collector.contains_registered(reachable));
    }

    #[test]
    fn state_prepass_closes_upvalues_before_invalidating_unreachable_handle() {
        let mut runtime = Runtime::new();
        let (
            reachable_handle,
            unreachable_handle,
            reachable_thread,
            unreachable_thread,
            unreachable_upvalue,
        ) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (main, collector, _) = parts.split_mut();
            let reachable_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("reachable child state is inserted");
            let unreachable_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("unreachable child state is inserted");
            let reachable_thread = collector.create(Thread::new());
            let unreachable_thread = collector.create(Thread::new());
            collector
                .with_mut(reachable_thread, |thread| {
                    thread.set_state_handle(reachable_handle);
                })
                .expect("reachable Thread remains registered");
            collector
                .with_mut(unreachable_thread, |thread| {
                    thread.set_state_handle(unreachable_handle);
                })
                .expect("unreachable Thread remains registered");
            main.with_resolved_state_mut(reachable_handle, |child| {
                child.current_thread = Some(reachable_thread);
            })
            .expect("reachable child resolves");
            let unreachable_upvalue = main
                .with_resolved_state_mut(unreachable_handle, |child| {
                    child.current_thread = Some(unreachable_thread);
                    child.push_number(41.0);
                    child
                        .find_or_create_upvalue(0, collector)
                        .expect("unreachable child owns one open Upvalue")
                })
                .expect("unreachable child resolves");
            main.push_value(Value::Thread(reachable_thread));
            (
                reachable_handle,
                unreachable_handle,
                reachable_thread,
                unreachable_thread,
                unreachable_upvalue,
            )
        };

        let report = runtime
            .collect_full_stw()
            .expect("state prepass can safely close an unreachable child");
        assert_eq!(report.coroutine_states_before, 2);
        assert_eq!(report.coroutine_states_after, 1);
        assert_eq!(report.swept_state_handles, vec![unreachable_handle]);
        assert_eq!(report.closed_open_upvalues, 1);
        assert_eq!(report.rejected_open_upvalue_edges, 0);
        assert_eq!(report.open_upvalue_owner_mismatches, 0);
        assert_eq!(report.open_upvalue_stack_values_missing, 0);

        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (main, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(reachable_thread));
        assert!(!collector.contains_registered(unreachable_thread));
        assert!(!collector.contains_registered(unreachable_upvalue));
        main.with_resolved_state_mut(reachable_handle, |_| ())
            .expect("reachable child handle remains valid");
        assert!(matches!(
            main.with_resolved_state_mut(unreachable_handle, |_| ()),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn full_collection_runs_unreachable_userdata_rust_destructor_once() {
        let drops = Rc::new(Cell::new(0));
        let mut runtime = Runtime::new();
        {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (_, collector, _) = parts.split_mut();
            let mut userdata = Userdata::new(std::mem::size_of::<DropProbe>());
            // SAFETY: the payload is initialized exactly once and Userdata's
            // typed destructor owns the matching drop_in_place operation.
            unsafe {
                userdata.write_typed(DropProbe(Rc::clone(&drops)));
            }
            collector.create(userdata);
        }

        let report = runtime
            .collect_full_stw()
            .expect("ordinary typed userdata can be swept internally");
        assert_eq!(report.collected_objects, 1);
        assert_eq!(drops.get(), 1);

        runtime
            .collect_full_stw()
            .expect("a later cycle does not redrop reclaimed userdata");
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn unsafe_root_diagnostic_rejects_sweep_without_destroying_objects() {
        let mut runtime = Runtime::new();
        let unreachable = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, _) = parts.split_mut();
            let unreachable = collector.create(Table::new());
            state.push_number(0.0);
            state.top = state.stack.size() + 1;
            unreachable
        };

        assert!(matches!(
            runtime.collect_full_stw(),
            Err(RuntimeFullCollectionError::UnsafeRootTrace { unsafe_gaps: 1, .. })
        ));

        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (state, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(unreachable));
        state.top = state.stack.size();
    }

    #[test]
    fn rejected_cross_collector_edge_prevents_any_local_sweep() {
        let mut runtime = Runtime::new();
        let mut foreign = GarbageCollector::new();
        let foreign_child = foreign.create(Table::new());
        let (local_root, local_unreachable) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, _) = parts.split_mut();
            let local_root = collector.create(Table::new());
            let local_unreachable = collector.create(Table::new());
            collector
                .with_mut(local_root, |table| {
                    table.set(&Value::Number(1.0), &Value::Table(foreign_child));
                })
                .expect("local root remains registered");
            state.push_value(Value::Table(local_root));
            (local_root, local_unreachable)
        };

        assert!(matches!(
            runtime.collect_full_stw(),
            Err(RuntimeFullCollectionError::UnsafeRootTrace {
                rejected_child_edges: 1,
                ..
            })
        ));
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(local_root));
        assert!(collector.contains_registered(local_unreachable));
    }

    #[test]
    fn full_collection_clears_and_sweeps_an_unreachable_weak_value() {
        let mut runtime = Runtime::new();
        let (weak_table, weak_value) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, strings) = parts.split_mut();
            let mode_key = strings.intern_bytes(collector, b"__mode");
            let mode_value = strings.intern_bytes(collector, b"v");
            let metatable = collector.create(Table::new());
            collector
                .with_mut(metatable, |table| {
                    table.set(&Value::String(mode_key), &Value::String(mode_value));
                })
                .expect("weak metatable remains registered");
            let weak_table = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set_metatable(Some(metatable));
                })
                .expect("weak table remains registered");
            let weak_value = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set(&Value::Number(1.0), &Value::Table(weak_value));
                })
                .expect("weak table remains registered");
            state.push_value(Value::Table(weak_table));
            (weak_table, weak_value)
        };

        runtime
            .collect_full_stw()
            .expect("weak-value cleanup is part of the atomic phase");
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(weak_table));
        assert!(!collector.contains_registered(weak_value));
        assert_eq!(
            collector
                .with_ref(weak_table, |table| table.get(&Value::Number(1.0)))
                .expect("weak table remains registered"),
            Value::Nil
        );
    }

    #[test]
    fn full_collection_retraces_and_retains_new_finalizers_for_delivery() {
        let mut runtime = Runtime::new();
        let finalizable = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (_, collector, strings) = parts.split_mut();
            let gc_key = strings.intern_bytes(collector, b"__gc");
            let metatable = collector.create(Table::new());
            collector
                .with_mut(metatable, |table| {
                    table.set(&Value::String(gc_key), &Value::Boolean(true));
                })
                .expect("finalizer metatable remains registered");
            let finalizable = collector.create(Userdata::new(0));
            collector
                .with_mut(finalizable, |userdata| {
                    userdata.set_metatable(Some(metatable));
                })
                .expect("finalizable userdata remains registered");
            finalizable
        };

        let report = runtime
            .collect_full_stw()
            .expect("new finalizer graph is retraced before sweep");
        assert_eq!(report.newly_discovered_finalizers, 1);
        assert_eq!(report.pending_finalizers_after, 1);
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(finalizable));
        assert_eq!(collector.pending_finalizer_count(), 1);
    }

    #[test]
    fn pending_finalizer_is_removed_as_a_weak_value_before_delivery() {
        let mut runtime = Runtime::new();
        let (weak_table, finalizable) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, strings) = parts.split_mut();
            let mode_key = strings.intern_bytes(collector, b"__mode");
            let weak_value_mode = strings.intern_bytes(collector, b"v");
            let gc_key = strings.intern_bytes(collector, b"__gc");
            let weak_metatable = collector.create(Table::new());
            collector
                .with_mut(weak_metatable, |table| {
                    table.set(&Value::String(mode_key), &Value::String(weak_value_mode));
                })
                .expect("weak metatable remains registered");
            let weak_table = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set_metatable(Some(weak_metatable));
                })
                .expect("weak table remains registered");
            state.push_value(Value::Table(weak_table));

            let finalizer_metatable = collector.create(Table::new());
            collector
                .with_mut(finalizer_metatable, |table| {
                    table.set(&Value::String(gc_key), &Value::Boolean(true));
                })
                .expect("finalizer metatable remains registered");
            let finalizable = collector.create(Userdata::new(0));
            collector
                .with_mut(finalizable, |userdata| {
                    userdata.set_metatable(Some(finalizer_metatable));
                })
                .expect("finalizable userdata remains registered");
            collector
                .with_mut(weak_table, |table| {
                    table.set(&Value::Number(1.0), &Value::Userdata(finalizable));
                })
                .expect("weak table remains registered");
            (weak_table, finalizable)
        };

        let report = runtime
            .collect_full_stw()
            .expect("pending finalizer is rooted but weak-value-dead");
        assert_eq!(report.newly_discovered_finalizers, 1);
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(finalizable));
        assert_eq!(
            collector
                .with_ref(weak_table, |table| table.get(&Value::Number(1.0)))
                .expect("weak table remains registered"),
            Value::Nil
        );
    }

    #[test]
    fn weak_key_cleanup_removes_the_key_but_marks_the_strong_value() {
        let mut runtime = Runtime::new();
        let (weak_table, weak_key, strong_value) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, strings) = parts.split_mut();
            let mode_key = strings.intern_bytes(collector, b"__mode");
            let weak_key_mode = strings.intern_bytes(collector, b"k");
            let metatable = collector.create(Table::new());
            collector
                .with_mut(metatable, |table| {
                    table.set(&Value::String(mode_key), &Value::String(weak_key_mode));
                })
                .expect("weak metatable remains registered");
            let weak_table = collector.create(Table::new());
            let weak_key = collector.create(Table::new());
            let strong_value = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set_metatable(Some(metatable));
                    table.set(&Value::Table(weak_key), &Value::Table(strong_value));
                })
                .expect("weak table remains registered");
            state.push_value(Value::Table(weak_table));
            (weak_table, weak_key, strong_value)
        };

        runtime
            .collect_full_stw()
            .expect("weak-key cleanup is sweep-safe");
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(weak_table));
        assert!(!collector.contains_registered(weak_key));
        assert!(collector.contains_registered(strong_value));
        assert_eq!(
            collector
                .with_ref(weak_table, |table| table.get(&Value::Table(weak_key)))
                .expect("weak table remains registered"),
            Value::Nil
        );
    }

    #[test]
    fn weak_key_and_value_mode_does_not_retain_either_side() {
        let mut runtime = Runtime::new();
        let (weak_table, weak_key, weak_value) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, strings) = parts.split_mut();
            let mode_key = strings.intern_bytes(collector, b"__mode");
            let weak_mode = strings.intern_bytes(collector, b"kv");
            let metatable = collector.create(Table::new());
            collector
                .with_mut(metatable, |table| {
                    table.set(&Value::String(mode_key), &Value::String(weak_mode));
                })
                .expect("weak metatable remains registered");
            let weak_table = collector.create(Table::new());
            let weak_key = collector.create(Table::new());
            let weak_value = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set_metatable(Some(metatable));
                    table.set(&Value::Table(weak_key), &Value::Table(weak_value));
                })
                .expect("weak table remains registered");
            state.push_value(Value::Table(weak_table));
            (weak_table, weak_key, weak_value)
        };

        runtime
            .collect_full_stw()
            .expect("weak key/value cleanup is sweep-safe");
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(weak_table));
        assert!(!collector.contains_registered(weak_key));
        assert!(!collector.contains_registered(weak_value));
    }

    #[test]
    fn finalized_weak_key_survives_resurrection_then_dies_after_root_removal() {
        let mut runtime = Runtime::new();
        let (weak_table, finalizable) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (state, collector, strings) = parts.split_mut();
            let mode_key = strings.intern_bytes(collector, b"__mode");
            let weak_key_mode = strings.intern_bytes(collector, b"k");
            let gc_key = strings.intern_bytes(collector, b"__gc");
            let weak_metatable = collector.create(Table::new());
            collector
                .with_mut(weak_metatable, |table| {
                    table.set(&Value::String(mode_key), &Value::String(weak_key_mode));
                })
                .expect("weak metatable remains registered");
            let weak_table = collector.create(Table::new());
            collector
                .with_mut(weak_table, |table| {
                    table.set_metatable(Some(weak_metatable));
                })
                .expect("weak table remains registered");
            state.push_value(Value::Table(weak_table));

            let finalizer_metatable = collector.create(Table::new());
            collector
                .with_mut(finalizer_metatable, |table| {
                    table.set(&Value::String(gc_key), &Value::Boolean(true));
                })
                .expect("finalizer metatable remains registered");
            let finalizable = collector.create(Userdata::new(0));
            collector
                .with_mut(finalizable, |userdata| {
                    userdata.set_metatable(Some(finalizer_metatable));
                })
                .expect("finalizable userdata remains registered");
            collector
                .with_mut(weak_table, |table| {
                    table.set(&Value::Userdata(finalizable), &Value::Boolean(true));
                })
                .expect("weak table remains registered");
            (weak_table, finalizable)
        };

        runtime
            .collect_full_stw()
            .expect("pending finalizer remains a live weak key");
        {
            let mut parts = runtime.parts_mut().expect("Runtime remains usable");
            let (state, collector, _) = parts.split_mut();
            assert_eq!(
                collector
                    .with_ref(weak_table, |table| {
                        table.get(&Value::Userdata(finalizable))
                    })
                    .expect("weak table remains registered"),
                Value::Boolean(true)
            );
            collector.clear_pending_finalizers();
            state.push_value(Value::Userdata(finalizable));
        }

        runtime
            .collect_full_stw()
            .expect("a reachable finalized userdata follows its live color");
        {
            let mut parts = runtime.parts_mut().expect("Runtime remains usable");
            let (state, collector, _) = parts.split_mut();
            assert!(collector.contains_registered(finalizable));
            assert_eq!(
                collector
                    .with_ref(weak_table, |table| {
                        table.get(&Value::Userdata(finalizable))
                    })
                    .expect("weak table remains registered"),
                Value::Boolean(true)
            );
            state.set_top(1);
        }

        runtime
            .collect_full_stw()
            .expect("a later unreachable finalized userdata is reclaimed");
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (_, collector, _) = parts.split_mut();
        assert!(!collector.contains_registered(finalizable));
        assert_eq!(
            collector
                .with_ref(weak_table, |table| {
                    table.get(&Value::Userdata(finalizable))
                })
                .expect("weak table remains registered"),
            Value::Nil
        );
    }

    #[test]
    fn finalizer_resurrection_graph_retraces_thread_state_edges() {
        let mut runtime = Runtime::new();
        let (child_handle, child_thread, finalizable) = {
            let mut parts = runtime.parts_mut().expect("Runtime parts are available");
            let (main, collector, strings) = parts.split_mut();
            let child_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("child state is inserted");
            let child_thread = collector.create(Thread::new());
            collector
                .with_mut(child_thread, |thread| {
                    thread.set_state_handle(child_handle);
                })
                .expect("child Thread remains registered");

            let gc_key = strings.intern_bytes(collector, b"__gc");
            let thread_key = strings.intern_bytes(collector, b"thread");
            let metatable = collector.create(Table::new());
            collector
                .with_mut(metatable, |table| {
                    table.set(&Value::String(gc_key), &Value::Boolean(true));
                    table.set(&Value::String(thread_key), &Value::Thread(child_thread));
                })
                .expect("finalizer metatable remains registered");
            let finalizable = collector.create(Userdata::new(0));
            collector
                .with_mut(finalizable, |userdata| {
                    userdata.set_metatable(Some(metatable));
                })
                .expect("finalizable userdata remains registered");
            (child_handle, child_thread, finalizable)
        };
        let report = runtime
            .collect_full_stw()
            .expect("the second canonical trace follows finalizer state edges");
        assert!(report.mark.traced_states.contains(&child_handle));
        assert!(!report.swept_state_handles.contains(&child_handle));
        let mut parts = runtime.parts_mut().expect("Runtime remains usable");
        let (main, collector, _) = parts.split_mut();
        assert!(collector.contains_registered(finalizable));
        assert!(collector.contains_registered(child_thread));
        main.with_resolved_state_mut(child_handle, |_| ())
            .expect("state reachable through the finalizer graph remains valid");
    }

    #[test]
    fn full_collection_enforces_runtime_phase_and_active_execution_gate() {
        let mut runtime = Runtime::new();
        runtime.active_executions = 1;
        assert!(matches!(
            runtime.collect_full_stw(),
            Err(RuntimeFullCollectionError::Access(
                RuntimeAccessError::ActiveExecutions { count: 1, .. }
            ))
        ));
        runtime.active_executions = 0;
        runtime.close().expect("owner-thread close succeeds");
        assert!(matches!(
            runtime.collect_full_stw(),
            Err(RuntimeFullCollectionError::Access(
                RuntimeAccessError::NotRunning {
                    phase: super::super::RuntimePhase::Closed,
                    ..
                }
            ))
        ));
    }
}

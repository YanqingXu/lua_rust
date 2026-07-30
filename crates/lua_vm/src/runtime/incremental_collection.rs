//! Runtime-owned incremental collection driver.

use std::collections::HashSet;

use lua_core::gc::collector::{GarbageCollector, IncrementalPhase};
use lua_core::string_pool::StringPool;

use super::full_collection::{RuntimeFullCollectionError, ensure_sweep_safe};
use super::root_trace::{IncrementalRootTrace, RuntimeRootSet};
use super::{NativeActivationStack, StateArena};

/// Result of one public incremental step request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RuntimeIncrementalStepReport {
    pub(super) cycle_finished: bool,
    pub(super) collected_objects: usize,
}

/// Advance one incremental cycle at a scheduler safe point.
pub(super) fn collect_incremental_step_at_safe_point(
    roots: RuntimeRootSet<'_>,
    state_arena: &mut StateArena,
    native_activations: &mut NativeActivationStack,
    gc: &mut GarbageCollector,
    strings: &mut StringPool,
    trace: &mut Option<IncrementalRootTrace>,
    size: i32,
) -> Result<RuntimeIncrementalStepReport, RuntimeFullCollectionError> {
    debug_assert!(state_arena.turn_borrow.is_none());
    if trace
        .as_ref()
        .is_some_and(|trace| !trace.matches_collector(gc))
    {
        *trace = None;
    }

    let budget = gc.incremental_work_budget(size);
    let run_to_completion = size >= 10_000;
    let result = loop {
        let report = match gc.incremental_phase() {
            IncrementalPhase::Pause => {
                *trace = Some(IncrementalRootTrace::begin(
                    roots,
                    state_arena,
                    native_activations,
                    gc,
                ));
                RuntimeIncrementalStepReport {
                    cycle_finished: false,
                    collected_objects: 0,
                }
            }
            IncrementalPhase::Propagate => {
                let active = trace
                    .as_mut()
                    .expect("propagate phase retains a Runtime root trace");
                active.step(state_arena, gc, budget);
                if active.is_complete(gc) {
                    gc.enter_incremental_atomic();
                }
                RuntimeIncrementalStepReport {
                    cycle_finished: false,
                    collected_objects: 0,
                }
            }
            IncrementalPhase::Atomic => {
                let active = trace
                    .as_mut()
                    .expect("atomic phase retains a Runtime root trace");
                active.atomic_rescan(roots, state_arena, native_activations, gc);
                active.drain(state_arena, gc);

                gc.reconcile_weak_table_modes();
                active.drain(state_arena, gc);

                let newly_discovered = gc.prepare_finalizable_userdata().len();
                if newly_discovered != 0 {
                    active.drain(state_arena, gc);
                }

                let mark = active.report(gc);
                ensure_sweep_safe(&mark)?;
                let reachable_states: HashSet<_> = mark.traced_states.iter().copied().collect();
                gc.clear_weak_table_entries();
                state_arena
                    .sweep_unreachable_owned(&reachable_states, gc)
                    .map_err(|source| RuntimeFullCollectionError::StatePrepass { source })?;
                gc.begin_incremental_sweep();

                RuntimeIncrementalStepReport {
                    cycle_finished: false,
                    collected_objects: 0,
                }
            }
            IncrementalPhase::Sweep => {
                gc.incremental_sweep_step(strings, budget);
                RuntimeIncrementalStepReport {
                    cycle_finished: false,
                    collected_objects: 0,
                }
            }
            IncrementalPhase::Finalize => {
                let collected_objects = gc.complete_incremental_cycle();
                *trace = None;
                RuntimeIncrementalStepReport {
                    cycle_finished: true,
                    collected_objects,
                }
            }
        };
        if report.cycle_finished || !run_to_completion {
            break report;
        }
    };
    gc.charge_incremental_step(size);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use lua_core::gc::collector::IncrementalPhase;
    use lua_core::gc::gc_object::GcObject;
    use lua_core::table::Table;
    use lua_core::thread::Thread;
    use lua_core::value::Value;

    use super::*;
    use crate::runtime::Runtime;
    use crate::state::LuaState;

    fn step_runtime(
        runtime: &mut Runtime,
        size: i32,
    ) -> Result<RuntimeIncrementalStepReport, RuntimeFullCollectionError> {
        let runtime_id = runtime.id;
        let main_handle = runtime.main_state_handle.expect("running Runtime");
        let global_root = runtime.global_root;
        let registry_root = runtime.registry_root;
        let fixed_strings = runtime.fixed_strings.clone();
        // SAFETY: the test holds the only Runtime borrow and no state turn is active.
        let storage = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        // SAFETY: no state turn or other arena borrow is active in this test.
        let arena = unsafe { &mut *storage.state_arena.get() };
        let activations = &mut storage.native_activations;
        let incremental_trace = &mut storage.incremental_trace;
        let (gc, strings) = storage.heap.parts_mut();
        collect_incremental_step_at_safe_point(
            RuntimeRootSet {
                runtime_id,
                main_handle,
                global_root,
                registry_root,
                fixed_strings: &fixed_strings,
            },
            arena,
            activations,
            gc,
            strings,
            incremental_trace,
            size,
        )
    }

    #[test]
    fn runtime_step_visits_every_phase_and_reclaims_only_at_bounded_sweep() {
        let mut runtime = Runtime::new();
        let (reachable, unreachable) = {
            let mut parts = runtime.parts_mut().unwrap();
            let (state, gc, _) = parts.split_mut();
            gc.set_step_multiplier(100);
            let reachable = gc.create(Table::new());
            let unreachable = gc.create(Table::new());
            state.push_value(Value::Table(reachable));
            (reachable, unreachable)
        };

        let mut phases = Vec::new();
        let mut finished = false;
        for _ in 0..10_000 {
            let report = step_runtime(&mut runtime, 0).unwrap();
            let mut parts = runtime.parts_mut().unwrap();
            let (_, gc, _) = parts.split_mut();
            phases.push(gc.incremental_phase());
            if report.cycle_finished {
                assert_eq!(report.collected_objects, 1);
                finished = true;
                break;
            }
        }
        assert!(finished, "bounded steps must eventually complete one cycle");
        assert!(phases.contains(&IncrementalPhase::Propagate));
        assert!(phases.contains(&IncrementalPhase::Atomic));
        assert!(phases.contains(&IncrementalPhase::Sweep));
        assert!(phases.contains(&IncrementalPhase::Finalize));
        assert_eq!(phases.last(), Some(&IncrementalPhase::Pause));

        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, _) = parts.split_mut();
        assert!(gc.contains_registered(reachable));
        assert!(!gc.contains_registered(unreachable));
    }

    #[test]
    fn large_step_runs_one_complete_cycle() {
        let mut runtime = Runtime::new();
        let unreachable = {
            let mut parts = runtime.parts_mut().unwrap();
            let (_, gc, _) = parts.split_mut();
            gc.create(Table::new())
        };

        let report = step_runtime(&mut runtime, 10_000).unwrap();
        assert!(report.cycle_finished);
        assert_eq!(report.collected_objects, 1);
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, _) = parts.split_mut();
        assert_eq!(gc.incremental_phase(), IncrementalPhase::Pause);
        assert!(!gc.contains_registered(unreachable));
    }

    #[test]
    fn table_barrier_publishes_thread_state_edge_to_runtime_queue() {
        let mut runtime = Runtime::new();
        let (parent, thread, child_handle, payload) = {
            let mut parts = runtime.parts_mut().unwrap();
            let (main, gc, _) = parts.split_mut();
            gc.set_step_multiplier(100);
            let parent = gc.create_root(Table::new());
            let payload = gc.create(Table::new());
            let child_handle = main.insert_coroutine_state(LuaState::new()).unwrap();
            let thread = gc.create(Thread::new());
            gc.with_mut(thread, |thread| thread.set_state_handle(child_handle))
                .unwrap();
            main.with_resolved_state_mut(child_handle, |child| {
                child.current_thread = Some(thread);
                child.push_value(Value::Table(payload));
            })
            .unwrap();
            (parent, thread, child_handle, payload)
        };

        // Drive only until the reachable parent is black.
        for _ in 0..10_000 {
            step_runtime(&mut runtime, 0).unwrap();
            let mut parts = runtime.parts_mut().unwrap();
            let (_, gc, _) = parts.split_mut();
            let parent_is_black = gc
                .with_ref(parent, |table| table.gc_header().is_black())
                .unwrap();
            if gc.incremental_phase() == IncrementalPhase::Propagate && parent_is_black {
                gc.with_mut(parent, |table| {
                    table.set(&Value::Number(1.0), &Value::Thread(thread));
                })
                .unwrap();
                break;
            }
        }

        let mut finished = false;
        for _ in 0..10_000 {
            if step_runtime(&mut runtime, 0).unwrap().cycle_finished {
                finished = true;
                break;
            }
        }
        assert!(finished);
        let mut parts = runtime.parts_mut().unwrap();
        let (main, gc, _) = parts.split_mut();
        assert!(gc.contains_registered(thread));
        assert!(gc.contains_registered(payload));
        main.with_resolved_state_mut(child_handle, |_| ())
            .expect("barrier-published Thread keeps its StateHandle target");
    }
}

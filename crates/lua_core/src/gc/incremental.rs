//! Runtime-driven incremental GC phase and accounting primitives.
//!
//! The collector owns colors, debt, controls, and the intrusive sweep cursor.
//! `lua_vm::Runtime` owns the cross-`StateArena` trace because `lua_core`
//! deliberately has no dependency on VM state storage.

use crate::gc::collector::{GarbageCollector, IncrementalPhase};
use crate::gc::header::GcObjectHeader;
use crate::gc::mark::MarkRootSeedReport;
use crate::state_handle::StateHandle;
use crate::string_pool::StringPool;
use crate::types::{GcColor, GcObjectType};

const MINIMUM_AUTOMATIC_THRESHOLD: usize = 64 * 1024;

impl GarbageCollector {
    /// Return the current incremental phase.
    pub fn incremental_phase(&self) -> IncrementalPhase {
        self.incremental_phase
    }

    /// Serial identifying the current Runtime/collector incremental trace.
    pub fn incremental_cycle_serial(&self) -> u64 {
        self.incremental_cycle_serial
    }

    /// Begin a cycle and seed collector-owned roots without tracing the
    /// Runtime's separate `StateArena`.
    pub fn begin_incremental_mark(&mut self) -> MarkRootSeedReport {
        debug_assert_eq!(self.incremental_phase, IncrementalPhase::Pause);
        self.gray_list.reserve(self.object_count);
        self.weak_tables.reserve(self.object_count);
        self.incremental_collected = 0;
        self.incremental_sweep_current = std::ptr::null_mut();
        self.incremental_sweep_previous = std::ptr::null_mut();
        self.incremental_cycle_serial = self.incremental_cycle_serial.wrapping_add(1);
        let report = self.begin_mark_only();
        self.incremental_phase = IncrementalPhase::Propagate;
        report
    }

    /// Publish that bounded root/object propagation reached a fixed point.
    pub fn enter_incremental_atomic(&mut self) {
        debug_assert_eq!(self.incremental_phase, IncrementalPhase::Propagate);
        self.incremental_phase = IncrementalPhase::Atomic;
    }

    /// Start bounded sweeping after Runtime atomic work and state pre-sweep.
    pub fn begin_incremental_sweep(&mut self) {
        debug_assert_eq!(self.incremental_phase, IncrementalPhase::Atomic);
        self.incremental_sweep_current = self.all_objects;
        self.incremental_sweep_previous = std::ptr::null_mut();
        self.incremental_phase = IncrementalPhase::Sweep;
    }

    /// Sweep at most `budget` intrusive-list nodes.
    ///
    /// Returns `(processed, collected, finished_sweep)`.
    pub fn incremental_sweep_step(
        &mut self,
        string_pool: &mut StringPool,
        budget: usize,
    ) -> (usize, usize, bool) {
        string_pool.bind_or_assert_owner(self.heap_id());
        debug_assert_eq!(self.incremental_phase, IncrementalPhase::Sweep);
        let mut processed = 0usize;
        let mut collected = 0usize;
        let budget = budget.max(1);

        while !self.incremental_sweep_current.is_null() && processed < budget {
            let object = self.incremental_sweep_current;
            let Some(live) = self.live_allocations.get(&(object as usize)).copied() else {
                self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                self.abort_incremental_cycle();
                return (processed, collected, false);
            };
            // SAFETY: the side table proves this cursor is a live list node.
            let next = unsafe { (*object).next() };
            // SAFETY: the same membership check permits reading header bits.
            let should_sweep = unsafe { (*object).is_white() && !(*object).is_fixed() };
            let open_upvalue = should_sweep
                && live.object_type == GcObjectType::Upval
                // SAFETY: the side-table tag proves the concrete layout.
                && unsafe { (&*object.cast::<crate::upvalue::Upvalue>()).is_open() };

            if should_sweep && !open_upvalue {
                if self.incremental_sweep_previous.is_null() {
                    self.all_objects = next;
                } else {
                    // SAFETY: previous is the retained live predecessor.
                    unsafe {
                        (*self.incremental_sweep_previous).set_next(next);
                    }
                }
                self.destroy_object(object, string_pool);
                collected = collected.saturating_add(1);
            } else {
                // Retained objects become candidates for the next cycle.
                // SAFETY: object remains registered and linked.
                unsafe {
                    (*object).set_color(GcColor::White);
                }
                self.incremental_sweep_previous = object;
            }

            self.incremental_sweep_current = next;
            processed = processed.saturating_add(1);
        }

        self.incremental_collected = self.incremental_collected.saturating_add(collected);
        let finished = self.incremental_sweep_current.is_null();
        if finished {
            self.weak_tables.clear();
            self.incremental_phase = IncrementalPhase::Finalize;
        }
        (processed, collected, finished)
    }

    /// Complete the Finalize phase after Runtime has scheduled callbacks.
    pub fn complete_incremental_cycle(&mut self) -> usize {
        debug_assert_eq!(self.incremental_phase, IncrementalPhase::Finalize);
        let collected = self.incremental_collected;
        self.last_completed_collected = collected;
        self.reset_incremental_state();
        self.update_automatic_threshold_after_cycle();
        collected
    }

    /// Conservatively abandon an active cycle without reclaiming more white
    /// objects. A later cycle resets every color and starts from all roots.
    pub fn abort_incremental_cycle(&mut self) {
        if self.incremental_phase == IncrementalPhase::Pause {
            return;
        }
        self.incremental_cycle_serial = self.incremental_cycle_serial.wrapping_add(1);
        self.reset_incremental_state();
    }

    fn reset_incremental_state(&mut self) {
        self.incremental_phase = IncrementalPhase::Pause;
        self.incremental_sweep_current = std::ptr::null_mut();
        self.incremental_sweep_previous = std::ptr::null_mut();
        self.incremental_collected = 0;
        self.gray_list.clear();
        self.weak_tables.clear();
        self.external_marked.clear();
    }

    /// Stop allocation-triggered progress. Explicit `step` remains available.
    pub fn stop_automatic(&mut self) {
        self.automatic_stopped = true;
    }

    /// Re-enable allocation-triggered progress.
    pub fn restart_automatic(&mut self) {
        self.automatic_stopped = false;
    }

    /// Whether automatic progress is stopped.
    pub fn is_automatic_stopped(&self) -> bool {
        self.automatic_stopped
    }

    /// Return the configured pause percentage.
    pub fn pause(&self) -> i32 {
        self.pause
    }

    /// Set the pause percentage and return its previous value.
    pub fn set_pause(&mut self, pause: i32) -> i32 {
        let previous = self.pause;
        self.pause = pause.max(0);
        previous
    }

    /// Return the configured step multiplier percentage.
    pub fn step_multiplier(&self) -> i32 {
        self.step_multiplier
    }

    /// Set the step multiplier and return its previous value.
    pub fn set_step_multiplier(&mut self, multiplier: i32) -> i32 {
        let previous = self.step_multiplier;
        self.step_multiplier = multiplier.max(0);
        previous
    }

    /// Current signed allocation/work debt.
    pub fn gc_debt_bytes(&self) -> i64 {
        self.gc_debt_bytes
    }

    /// Diagnostic threshold used once automatic checkpoints are enabled.
    pub fn automatic_threshold_bytes(&self) -> usize {
        self.automatic_threshold_bytes
    }

    /// Convert Lua's step argument into a bounded object/state work budget.
    pub fn incremental_work_budget(&self, size: i32) -> usize {
        let requested_kilobytes = u64::from(size.max(0) as u32).saturating_add(1);
        let multiplier = u64::from(self.step_multiplier.max(1) as u32);
        let scaled = requested_kilobytes.saturating_mul(multiplier);
        let whole_kilobytes = scaled / 100;
        whole_kilobytes
            .max(1)
            .min(i32::MAX as u64)
            .try_into()
            .unwrap_or(usize::MAX)
    }

    /// Charge the byte-scaled work requested by one public `step`.
    pub fn charge_incremental_step(&mut self, size: i32) {
        let requested_kilobytes = u64::from(size.max(0) as u32).saturating_add(1);
        let multiplier = u64::from(self.step_multiplier.max(1) as u32);
        let scaled_percent = requested_kilobytes.saturating_mul(multiplier);
        let bytes = scaled_percent.saturating_mul(1024) / 100;
        self.subtract_gc_debt(bytes.min(usize::MAX as u64) as usize);
    }

    pub(crate) fn add_gc_debt(&mut self, bytes: usize) {
        self.gc_debt_bytes = self
            .gc_debt_bytes
            .saturating_add(i64::try_from(bytes).unwrap_or(i64::MAX));
    }

    pub(crate) fn subtract_gc_debt(&mut self, bytes: usize) {
        self.gc_debt_bytes = self
            .gc_debt_bytes
            .saturating_sub(i64::try_from(bytes).unwrap_or(i64::MAX));
    }

    fn update_automatic_threshold_after_cycle(&mut self) {
        let pause = self.pause.max(100) as usize;
        let paused = self.total_memory.saturating_mul(pause) / 100;
        self.automatic_threshold_bytes = paused.max(MINIMUM_AUTOMATIC_THRESHOLD);
        self.gc_debt_bytes = i64::try_from(self.total_memory)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(self.automatic_threshold_bytes).unwrap_or(i64::MAX));
    }

    /// Publish the complete initial graph of an allocation created during an
    /// active cycle.
    pub(crate) fn publish_new_allocation(
        &mut self,
        object: *mut GcObjectHeader,
        object_type: GcObjectType,
    ) {
        if self.incremental_phase == IncrementalPhase::Sweep {
            self.abort_incremental_cycle();
            return;
        }
        if matches!(object_type, GcObjectType::Thread | GcObjectType::Upval) {
            // Their child graph can publish StateHandles, which lua_core
            // cannot enqueue. Restarting is conservative and uncommon.
            self.abort_incremental_cycle();
            return;
        }
        // SAFETY: create registered the exact concrete allocation and keeps it
        // alive for this call.
        unsafe {
            self.trace_object_children_for_barrier(object, object_type);
        }
    }

    /// Unified post-write mutation barrier used by every checked mutable
    /// collector borrow.
    pub(crate) fn after_managed_mutation(
        &mut self,
        owner: *mut GcObjectHeader,
        object_type: GcObjectType,
        state_edge_changed: bool,
    ) {
        if self.incremental_phase == IncrementalPhase::Pause {
            return;
        }
        if self.incremental_phase == IncrementalPhase::Sweep
            || (self.incremental_phase == IncrementalPhase::Propagate && state_edge_changed)
        {
            self.abort_incremental_cycle();
            return;
        }
        // SAFETY: `with_mut` validated the owner and the callback cannot
        // destroy it. Only black owners need a forward barrier.
        if unsafe { !(*owner).is_black() } {
            return;
        }
        // SAFETY: the validated type tag selects the concrete trace layout.
        unsafe {
            self.trace_object_children_for_barrier(owner, object_type);
        }
    }

    /// Return the cross-`StateArena` edge carried by one managed object.
    pub(crate) fn managed_state_edge(
        &self,
        owner: *mut GcObjectHeader,
        object_type: GcObjectType,
    ) -> Option<StateHandle> {
        // SAFETY: caller has already validated the concrete owner allocation.
        unsafe {
            match object_type {
                GcObjectType::Thread => (&*owner.cast::<crate::thread::Thread>()).state_handle(),
                GcObjectType::Upval => (&*owner.cast::<crate::upvalue::Upvalue>())
                    .open_location()
                    .map(|(state, _)| state),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::header::GcObjectHeader;
    use crate::table::Table;
    use crate::value::Value;

    #[test]
    fn bounded_sweep_advances_one_intrusive_node_per_unit() {
        let mut gc = GarbageCollector::new();
        let mut strings = StringPool::new();
        let root = gc.create_root(Table::new());
        let garbage_a = gc.create(Table::new());
        let garbage_b = gc.create(Table::new());

        gc.begin_incremental_mark();
        gc.propagate_marks();
        gc.enter_incremental_atomic();
        gc.begin_incremental_sweep();

        let objects_before = gc.object_count();
        let (processed, collected, finished) = gc.incremental_sweep_step(&mut strings, 1);
        assert_eq!(processed, 1);
        assert!(collected <= 1);
        assert!(!finished);
        assert!(gc.object_count() >= objects_before - 1);

        while gc.incremental_phase() == IncrementalPhase::Sweep {
            gc.incremental_sweep_step(&mut strings, 1);
        }
        assert_eq!(gc.incremental_phase(), IncrementalPhase::Finalize);
        assert_eq!(gc.complete_incremental_cycle(), 2);
        assert!(gc.contains_registered(root));
        assert!(!gc.contains_registered(garbage_a));
        assert!(!gc.contains_registered(garbage_b));
    }

    #[test]
    fn checked_mutation_marks_a_new_black_to_white_edge() {
        let mut gc = GarbageCollector::new();
        let owner = gc.create_root(Table::new());
        let child = gc.create(Table::new());
        gc.begin_incremental_mark();
        gc.propagate_marks();

        let child_header = child.as_ptr().cast_mut().cast::<GcObjectHeader>();
        // SAFETY: child remains registered.
        assert!(unsafe { (*child_header).is_white() });
        gc.with_mut(owner, |table| {
            table.set(&Value::Number(1.0), &Value::Table(child));
        })
        .unwrap();
        // SAFETY: the post-write barrier publishes child as gray; Runtime
        // consumes it as a bounded work unit.
        assert!(unsafe { !(*child_header).is_white() });
        gc.propagate_marks();
        // SAFETY: the standalone test drains the collector-only queue.
        assert!(unsafe { (*child_header).is_black() });
        assert_eq!(gc.incremental_phase(), IncrementalPhase::Propagate);
    }

    #[test]
    fn sweep_time_mutation_abandons_the_cursor_before_reclaiming_new_edge() {
        let mut gc = GarbageCollector::new();
        let owner = gc.create_root(Table::new());
        let child = gc.create(Table::new());
        gc.begin_incremental_mark();
        gc.propagate_marks();
        gc.enter_incremental_atomic();
        gc.begin_incremental_sweep();

        gc.with_mut(owner, |table| {
            table.set(&Value::Number(1.0), &Value::Table(child));
        })
        .unwrap();
        assert_eq!(gc.incremental_phase(), IncrementalPhase::Pause);
        assert!(gc.contains_registered(child));
    }

    #[test]
    fn active_cycle_allocation_is_black_and_publishes_its_initial_graph() {
        let mut gc = GarbageCollector::new();
        let child = gc.create(Table::new());
        gc.begin_incremental_mark();
        let mut parent = Table::new();
        parent.set(&Value::Number(1.0), &Value::Table(child));
        let parent = gc.create(parent);

        let parent_header = parent.as_ptr().cast_mut().cast::<GcObjectHeader>();
        let child_header = child.as_ptr().cast_mut().cast::<GcObjectHeader>();
        // SAFETY: both allocations remain registered.
        assert!(unsafe { (*parent_header).is_black() });
        // SAFETY: initial-graph publication makes the child non-white.
        assert!(unsafe { !(*child_header).is_white() });
    }

    #[test]
    fn post_write_barrier_preserves_weak_value_semantics() {
        let mut gc = GarbageCollector::new();
        let mut strings = StringPool::new();
        let mode_key = strings.intern_bytes(&mut gc, b"__mode");
        let mode_value = strings.intern_bytes(&mut gc, b"v");
        let metatable = gc.create(Table::new());
        gc.with_mut(metatable, |table| {
            table.set(&Value::String(mode_key), &Value::String(mode_value));
        })
        .unwrap();
        let weak = gc.create_root(Table::new());
        gc.with_mut(weak, |table| table.set_metatable(Some(metatable)))
            .unwrap();
        let value = gc.create(Table::new());

        gc.begin_incremental_mark();
        gc.propagate_marks();
        gc.with_mut(weak, |table| {
            table.set(&Value::Number(1.0), &Value::Table(value));
        })
        .unwrap();

        let value_header = value.as_ptr().cast_mut().cast::<GcObjectHeader>();
        // SAFETY: weak-value insertion must not strengthen the child.
        assert!(unsafe { (*value_header).is_white() });
        assert_eq!(gc.incremental_phase(), IncrementalPhase::Propagate);
    }

    #[test]
    fn pause_step_multiplier_and_debt_follow_oracle_controls() {
        let mut gc = GarbageCollector::new();
        assert_eq!(gc.pause(), 200);
        assert_eq!(gc.step_multiplier(), 200);
        assert_eq!(gc.set_pause(150), 200);
        assert_eq!(gc.set_step_multiplier(50), 200);
        assert_eq!(gc.incremental_work_budget(0), 1);

        let debt_before = gc.gc_debt_bytes();
        gc.create(Table::new());
        assert!(gc.gc_debt_bytes() > debt_before);
        gc.charge_incremental_step(0);
        assert!(gc.gc_debt_bytes() < debt_before);

        assert!(!gc.is_automatic_stopped());
        gc.stop_automatic();
        assert!(gc.is_automatic_stopped());
        gc.restart_automatic();
        assert!(!gc.is_automatic_stopped());
    }
}

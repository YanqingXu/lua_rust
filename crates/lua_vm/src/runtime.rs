//! Transitional owner for one Lua runtime.
//!
//! `Runtime` is the first ownership boundary for the Rust VM. It keeps the
//! collector, string pool, and generational state arena in a pinned heap
//! allocation. It owns a boxed main `LuaState`; the arena owns every coroutine
//! state. Legacy service and arena backpointers are installed only after their
//! targets have stable addresses. Callers can borrow the three execution parts
//! together through `Runtime::parts_mut`; the returned guard records both the
//! Runtime execution and main-state arena borrow.
//!
//! M1.8 adds explicit deterministic reclamation for shutdown only.
//! `Runtime::close` drains coroutine states, closes collector-validated open
//! Upvalues while their stacks are alive, detaches the main state, clears
//! Runtime roots, and calls the collector's non-collecting `destroy_all` path.
//! That path destroys non-fixed Threads first, other non-fixed objects second,
//! and fixed objects last while a live StringPool removes every String index.
//! Owner-thread `Drop` uses the same implementation.
//!
//! This remains a partial shutdown substrate: close-time Lua `__gc`,
//! library-specific IO/resource hooks, native-module ordering, and allocator
//! live-byte accounting are explicit report/RFC debt. No shutdown path calls
//! `collect`, `sweep`, or the legacy fixed-preserving `clear_all`, and live-VM
//! destructive collection remains disabled.

use std::marker::{PhantomData, PhantomPinned};
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, ThreadId};

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
pub use lua_core::state_handle::{RuntimeId, StateHandle};
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use thiserror::Error;

use crate::state::LuaState;
use crate::state::lua_state::LuaStateShutdownReport;

mod root_trace;
pub use root_trace::{
    MarkOnlyReport, RootEdgeCount, RuntimeRootKind, StateTraceFailure, UnresolvedObjectEdge,
    UnsafeTraceGap, UnsafeTraceGapKind,
};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

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
    /// Close was requested while execution borrows are active.
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
}

/// Result of explicit Runtime teardown.
///
/// M1.8 reclaims Rust-owned states and collector allocations, but deliberately
/// does not invoke Lua-visible `__gc` or library-specific close callbacks.
/// Those semantic gaps are exposed as debt fields rather than hidden behind
/// successful memory reclamation.
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
    /// Coroutine states still retained after close.
    pub remaining_coroutine_states: usize,
    /// Transient gray/weak/finalizer/external collector queue entries.
    pub remaining_collector_queue_entries: usize,
    /// Coroutine states drained and generation-invalidated by the first close.
    pub drained_coroutine_states: usize,
    /// Open Upvalues safely closed while their owner state stacks were alive.
    pub closed_open_upvalues: usize,
    /// Invalid/cross-collector Upvalue list edges rejected before dereference.
    pub rejected_open_upvalue_edges: usize,
    /// Repeated Upvalue list nodes rejected as cycles.
    pub open_upvalue_cycles: usize,
    /// Open Upvalues whose raw owner-Stack pointer did not match the state.
    pub open_upvalue_owner_mismatches: usize,
    /// Open Upvalues whose stack slot was outside the active initialized view.
    pub open_upvalue_stack_values_missing: usize,
    /// Concrete collector allocations destroyed by the first close.
    pub destroyed_objects: usize,
    /// Non-fixed Thread allocations destroyed in the first GC teardown pass.
    pub destroyed_threads: usize,
    /// Fixed allocations destroyed in the final GC teardown pass.
    pub destroyed_fixed_objects: usize,
    /// Pending-finalizer queue entries discarded without Lua callback.
    pub pending_lua_finalizers_discarded: usize,
    /// Capability debt: close-time Lua `__gc` callbacks are not implemented.
    pub lua_gc_callback_debt: bool,
    /// Capability debt: library-specific IO/resource drain hooks are not run.
    pub io_resource_drain_debt: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RuntimeShutdownSummary {
    drained_coroutine_states: usize,
    state_shutdown: LuaStateShutdownReport,
    destroyed_objects: usize,
    destroyed_threads: usize,
    destroyed_fixed_objects: usize,
    pending_lua_finalizers_discarded: usize,
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
    /// Resolution attempted to recreate the caller's current mutable state.
    #[error("cannot resolve the currently mutably borrowed LuaState")]
    CurrentState,
    /// A direct owner borrow did not match the registered state allocation.
    #[error("registered state pointer did not match the Runtime owner")]
    PointerMismatch,
    /// A non-owning slot was passed to an owned-state removal operation.
    #[error("state slot {slot} is not owned by the StateArena")]
    NotOwned {
        /// Non-owning slot index.
        slot: usize,
    },
    /// An owned slot was passed to the external-state detach operation.
    #[error("state slot {slot} is owned by the StateArena, not externally boxed")]
    NotExternal {
        /// Owned slot index.
        slot: usize,
    },
}

struct StateSlot {
    generation: u64,
    state: Option<NonNull<LuaState>>,
    owned: bool,
    borrowed: bool,
}

/// Runtime-owned generational arena for Lua coroutine states.
///
/// Slots contain stable `NonNull<LuaState>` addresses rather than references.
/// All resolution validates runtime, slot, generation, occupancy, current-state
/// identity, and exclusive-borrow state. References are created only inside a
/// higher-ranked closure and therefore cannot escape the resolution scope.
pub struct StateArena {
    runtime_id: RuntimeId,
    slots: Vec<StateSlot>,
    free_slots: Vec<usize>,
    live_owned_states: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StateArenaDrainReport {
    drained_owned_states: usize,
    state_shutdown: LuaStateShutdownReport,
}

impl StateArena {
    fn new(runtime_id: RuntimeId) -> Self {
        Self {
            runtime_id,
            slots: Vec::new(),
            free_slots: Vec::new(),
            live_owned_states: 0,
        }
    }

    fn reserve_slot(&mut self) -> StateHandle {
        let slot = if let Some(slot) = self.free_slots.pop() {
            slot
        } else {
            self.slots.push(StateSlot {
                generation: 1,
                state: None,
                owned: false,
                borrowed: false,
            });
            self.slots.len() - 1
        };
        StateHandle::new(self.runtime_id, slot, self.slots[slot].generation)
    }

    fn attach_external(&mut self, state: NonNull<LuaState>) -> StateHandle {
        let handle = self.reserve_slot();
        let slot = &mut self.slots[handle.slot()];
        slot.state = Some(state);
        slot.owned = false;
        handle
    }

    fn insert_owned(&mut self, mut state: Box<LuaState>) -> StateHandle {
        let handle = self.reserve_slot();
        let arena = NonNull::from(&mut *self);
        state.attach_runtime_state(handle, arena);
        let state =
            NonNull::new(Box::into_raw(state)).expect("Box::into_raw never returns a null pointer");

        let slot = &mut self.slots[handle.slot()];
        slot.state = Some(state);
        slot.owned = true;
        self.live_owned_states += 1;
        handle
    }

    fn validate(
        &self,
        handle: StateHandle,
    ) -> Result<(usize, NonNull<LuaState>), StateResolveError> {
        if handle.runtime_id() != self.runtime_id {
            return Err(StateResolveError::ForeignRuntime {
                expected: self.runtime_id,
                actual: handle.runtime_id(),
            });
        }
        let Some(slot) = self.slots.get(handle.slot()) else {
            return Err(StateResolveError::InvalidSlot {
                slot: handle.slot(),
            });
        };
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
            arena_ref.slots[handle.slot()].borrowed = true;
            state
        };
        let mut borrow = StateBorrow {
            arena,
            handle,
            state,
        };
        Ok(borrow.with_mut(f))
    }

    fn begin_direct_borrow(
        &mut self,
        handle: StateHandle,
        expected_state: NonNull<LuaState>,
    ) -> Result<(), StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        if state != expected_state {
            return Err(StateResolveError::PointerMismatch);
        }
        let slot = &mut self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        slot.borrowed = true;
        Ok(())
    }

    fn release(&mut self, handle: StateHandle) -> bool {
        let Ok((slot_index, _)) = self.validate(handle) else {
            return false;
        };
        let was_borrowed = self.slots[slot_index].borrowed;
        self.slots[slot_index].borrowed = false;
        was_borrowed
    }

    fn detach_external(
        &mut self,
        handle: StateHandle,
        expected_state: NonNull<LuaState>,
    ) -> Result<(), StateResolveError> {
        let slot_index = self.validate_external_detach(handle, expected_state)?;
        self.vacate_slot(slot_index);
        Ok(())
    }

    fn validate_external_detach(
        &self,
        handle: StateHandle,
        expected_state: NonNull<LuaState>,
    ) -> Result<usize, StateResolveError> {
        let (slot_index, state) = self.validate(handle)?;
        if state != expected_state {
            return Err(StateResolveError::PointerMismatch);
        }
        let slot = &self.slots[slot_index];
        if slot.borrowed {
            return Err(StateResolveError::AlreadyBorrowed {
                slot: slot_index,
                generation: slot.generation,
            });
        }
        if slot.owned {
            return Err(StateResolveError::NotExternal { slot: slot_index });
        }
        Ok(slot_index)
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
        if !slot.owned {
            return Err(StateResolveError::NotOwned { slot: slot_index });
        }

        self.vacate_slot(slot_index);
        self.live_owned_states -= 1;
        // SAFETY: owned slots are created exclusively with Box::into_raw,
        // removed at most once, and have just been made vacant.
        unsafe {
            drop(Box::from_raw(state.as_ptr()));
        }
        Ok(())
    }

    /// Drain every arena-owned coroutine state in deterministic slot order.
    ///
    /// All occupied owned slots are preflighted for outstanding borrows before
    /// any mutation. Each slot generation is invalidated before its Box is
    /// dropped, and every state closes collector-validated open Upvalues while
    /// the collector heap is still alive.
    fn drain_owned(
        &mut self,
        gc: &GarbageCollector,
    ) -> Result<StateArenaDrainReport, StateResolveError> {
        self.validate_owned_drain()?;

        let mut report = StateArenaDrainReport::default();
        for slot_index in 0..self.slots.len() {
            let Some(state) = self.slots[slot_index].state else {
                continue;
            };
            if !self.slots[slot_index].owned {
                continue;
            }

            self.vacate_slot(slot_index);
            self.live_owned_states -= 1;
            // SAFETY: each owned slot originates from exactly one Box::into_raw
            // and was made vacant before ownership is reconstructed here.
            let mut state = unsafe { Box::from_raw(state.as_ptr()) };
            report
                .state_shutdown
                .merge(state.prepare_for_runtime_shutdown(gc));
            report.drained_owned_states += 1;
            drop(state);
        }

        debug_assert_eq!(self.live_owned_states, 0);
        Ok(report)
    }

    fn validate_owned_drain(&self) -> Result<(), StateResolveError> {
        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot.owned && slot.state.is_some() && slot.borrowed {
                return Err(StateResolveError::AlreadyBorrowed {
                    slot: slot_index,
                    generation: slot.generation,
                });
            }
        }
        Ok(())
    }

    fn vacate_slot(&mut self, slot_index: usize) {
        let slot = &mut self.slots[slot_index];
        slot.state = None;
        slot.owned = false;
        slot.borrowed = false;
        slot.generation = next_generation(slot.generation);
        self.free_slots.push(slot_index);
    }

    fn live_owned_state_count(&self) -> usize {
        self.live_owned_states
    }
}

impl Drop for StateArena {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            let Some(state) = slot.state.take() else {
                continue;
            };
            if slot.owned {
                // SAFETY: every owned occupied slot came from exactly one
                // Box::into_raw and StateArena is its unique final owner.
                unsafe {
                    drop(Box::from_raw(state.as_ptr()));
                }
            }
            slot.borrowed = false;
        }
        self.live_owned_states = 0;
    }
}

struct StateBorrow {
    arena: NonNull<StateArena>,
    handle: StateHandle,
    state: NonNull<LuaState>,
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
        // pinned RuntimeHeap and its StateArena therefore still exist.
        let released = unsafe { arena.as_mut() }.release(self.handle);
        debug_assert!(released);
    }
}

fn next_generation(generation: u64) -> u64 {
    let next = generation.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

impl LuaState {
    pub(crate) fn attach_runtime_state(&mut self, handle: StateHandle, arena: NonNull<StateArena>) {
        self.state_handle = Some(handle);
        self.state_arena = Some(arena);
    }

    /// Return this state's runtime-scoped generational handle.
    pub fn state_handle(&self) -> Option<StateHandle> {
        self.state_handle
    }

    /// Move a coroutine state into this state's Runtime-owned arena.
    ///
    /// Standalone `LuaState` values are intentionally rejected: coroutine
    /// states require a Runtime owner so their allocation cannot leak.
    pub fn insert_coroutine_state(
        &mut self,
        state: LuaState,
    ) -> Result<StateHandle, StateResolveError> {
        let mut arena = self
            .state_arena
            .ok_or(StateResolveError::ArenaUnavailable)?;
        // SAFETY: Runtime attaches only pointers to its pinned StateArena.
        // The current LuaState is alive for this call, so the Runtime and arena
        // cannot be dropped while the child Box is transferred.
        Ok(unsafe { arena.as_mut() }.insert_owned(Box::new(state)))
    }

    /// Resolve another state and confine its mutable reference to `f`.
    ///
    /// The target must belong to the same Runtime and must differ from this
    /// currently borrowed state. Nested execution borrows are tracked by slot,
    /// so a caller or other active coroutine cannot be aliased.
    pub fn with_resolved_state_mut<R>(
        &mut self,
        handle: StateHandle,
        f: impl for<'state> FnOnce(&'state mut LuaState) -> R,
    ) -> Result<R, StateResolveError> {
        let arena = self
            .state_arena
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let current_state = NonNull::from(&mut *self);
        // SAFETY: state_arena is installed only from the pinned RuntimeHeap.
        // StateArena validates target identity and guards the mutable borrow
        // until the higher-ranked closure returns.
        StateArena::with_state_mut(arena, handle, current_state, f)
    }
}

/// Stable allocation for services referenced by transitional raw backpointers.
struct RuntimeHeap {
    state_arena: StateArena,
    gc: GarbageCollector,
    string_pool: StringPool,
    _pin: PhantomPinned,
}

impl RuntimeHeap {
    fn new(runtime_id: RuntimeId) -> Self {
        Self {
            state_arena: StateArena::new(runtime_id),
            gc: GarbageCollector::new(),
            string_pool: StringPool::new(),
            _pin: PhantomPinned,
        }
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
    main_state: Option<Box<LuaState>>,
    main_state_handle: Option<StateHandle>,
    global_root: Option<GcRef<Table>>,
    registry_root: Option<GcRef<Table>>,
    shutdown_summary: Option<RuntimeShutdownSummary>,
    heap: Pin<Box<RuntimeHeap>>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Runtime {
    /// Create a running runtime with a rooted global table and main state.
    ///
    /// The legacy `LuaState::gc` and `LuaState::string_pool` pointers target
    /// the pinned heap and stay stable even when the `Runtime` value moves.
    pub fn new() -> Self {
        let id = RuntimeId::new(NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed));
        let mut heap = Box::pin(RuntimeHeap::new(id));

        // SAFETY: `RuntimeHeap` is pinned for the remainder of the Runtime's
        // life. We take field addresses and never move out of the heap.
        let heap_mut = unsafe { Pin::get_unchecked_mut(heap.as_mut()) };
        let gc_ptr = &mut heap_mut.gc as *mut GarbageCollector;
        let string_pool_ptr = &mut heap_mut.string_pool as *mut StringPool;
        let global_root = heap_mut.gc.create_root(Table::new());
        let registry_root = heap_mut.gc.create(Table::new());

        let mut main_state = Box::new(LuaState::with_global_table(global_root));
        main_state.registry = Some(registry_root);
        main_state.gc = Some(gc_ptr);
        main_state.string_pool = Some(string_pool_ptr);
        let main_state_ptr = NonNull::from(main_state.as_mut());
        let main_state_handle = heap_mut.state_arena.attach_external(main_state_ptr);
        main_state
            .attach_runtime_state(main_state_handle, NonNull::from(&mut heap_mut.state_arena));

        Self {
            id,
            owner_thread: thread::current().id(),
            phase: RuntimePhase::Running,
            active_executions: 0,
            main_state: Some(main_state),
            main_state_handle: Some(main_state_handle),
            global_root: Some(global_root),
            registry_root: Some(registry_root),
            shutdown_summary: None,
            heap,
            _not_send_or_sync: PhantomData,
        }
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

    /// Return the boxed main state while the runtime remains open.
    pub fn main_state(&self) -> Option<&LuaState> {
        self.main_state.as_deref()
    }

    /// Return coroutine states currently retained by the StateArena.
    pub fn live_coroutine_state_count(&self) -> usize {
        self.heap
            .as_ref()
            .get_ref()
            .state_arena
            .live_owned_state_count()
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

        let state =
            self.main_state
                .as_deref_mut()
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;
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
        let state_ptr = NonNull::from(&mut *state);
        heap.state_arena
            .begin_direct_borrow(state_handle, state_ptr)
            .map_err(|source| RuntimeAccessError::StateArena {
                runtime_id: self.id,
                source,
            })?;
        self.active_executions = next_active_executions;

        Ok(RuntimePartsMut {
            state,
            gc: &mut heap.gc,
            string_pool: &mut heap.string_pool,
            state_arena: NonNull::from(&mut heap.state_arena),
            state_handle,
            active_executions: &mut self.active_executions,
        })
    }

    /// Deterministically reclaim Runtime-owned Rust states and GC allocations.
    ///
    /// Close is idempotent after the first successful call. This M1.8 partial
    /// substrate closes validated open Upvalues, invalidates every StateArena
    /// slot, destroys non-fixed Threads before other non-fixed objects and
    /// fixed objects last, and empties the live StringPool. It deliberately
    /// does not run Lua-visible `__gc` or library-specific resource callbacks;
    /// those capability debts are explicit in the returned report.
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
                self.phase = RuntimePhase::Closing;
                let summary = match self.shutdown_contents() {
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

    fn validate_shutdown_state(&self) -> Result<(), StateResolveError> {
        let state = self
            .main_state
            .as_deref()
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let handle = self
            .main_state_handle
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let state_pointer = NonNull::from(state);
        let heap = self.heap.as_ref().get_ref();
        heap.state_arena
            .validate_external_detach(handle, state_pointer)?;
        heap.state_arena.validate_owned_drain()
    }

    fn shutdown_contents(&mut self) -> Result<RuntimeShutdownSummary, StateResolveError> {
        let arena_report = {
            // SAFETY: RuntimeHeap remains pinned and shutdown has exclusive
            // owner-thread access with no execution guard.
            let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
            heap.state_arena.drain_owned(&heap.gc)?
        };

        let mut state_shutdown = arena_report.state_shutdown;
        state_shutdown.merge(self.detach_main_state_for_shutdown()?);

        let global_root = self.global_root.take();
        let registry_root = self.registry_root.take();
        // SAFETY: same pinned/exclusive shutdown invariant as above. All state
        // allocations and their service backpointers have already been
        // detached before collector allocations are destroyed.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        if let Some(global_root) = global_root {
            heap.gc.remove_root(global_root);
        }
        if let Some(registry_root) = registry_root {
            heap.gc.remove_root(registry_root);
        }
        let gc_report = heap.gc.destroy_all(&mut heap.string_pool);

        Ok(RuntimeShutdownSummary {
            drained_coroutine_states: arena_report.drained_owned_states,
            state_shutdown,
            destroyed_objects: gc_report.destroyed_objects(),
            destroyed_threads: gc_report.destroyed_threads,
            destroyed_fixed_objects: gc_report.destroyed_fixed,
            pending_lua_finalizers_discarded: gc_report.pending_finalizers_discarded,
        })
    }

    fn close_report(&self, already_closed: bool) -> RuntimeCloseReport {
        let heap = self.heap.as_ref().get_ref();
        let remaining_objects = heap.gc.object_count();
        let remaining_coroutine_states = heap.state_arena.live_owned_state_count();
        let remaining_collector_queue_entries = heap.gc.transient_queue_entry_count();
        let summary = self.shutdown_summary.unwrap_or_default();
        RuntimeCloseReport {
            runtime_id: self.id,
            already_closed,
            heap_reclamation_deferred: remaining_objects != 0
                || heap.gc.root_count() != 0
                || !heap.string_pool.is_empty()
                || remaining_coroutine_states != 0
                || remaining_collector_queue_entries != 0,
            remaining_objects,
            remaining_roots: heap.gc.root_count(),
            remaining_interned_strings: heap.string_pool.len(),
            remaining_estimated_bytes: heap.gc.total_memory(),
            remaining_coroutine_states,
            remaining_collector_queue_entries,
            drained_coroutine_states: summary.drained_coroutine_states,
            closed_open_upvalues: summary.state_shutdown.closed_open_upvalues,
            rejected_open_upvalue_edges: summary.state_shutdown.rejected_open_upvalue_edges,
            open_upvalue_cycles: summary.state_shutdown.open_upvalue_cycles,
            open_upvalue_owner_mismatches: summary.state_shutdown.open_upvalue_owner_mismatches,
            open_upvalue_stack_values_missing: summary
                .state_shutdown
                .open_upvalue_stack_values_missing,
            destroyed_objects: summary.destroyed_objects,
            destroyed_threads: summary.destroyed_threads,
            destroyed_fixed_objects: summary.destroyed_fixed_objects,
            pending_lua_finalizers_discarded: summary.pending_lua_finalizers_discarded,
            lua_gc_callback_debt: true,
            io_resource_drain_debt: true,
        }
    }

    fn detach_main_state_for_shutdown(
        &mut self,
    ) -> Result<LuaStateShutdownReport, StateResolveError> {
        let state = self
            .main_state
            .as_deref_mut()
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let handle = self
            .main_state_handle
            .ok_or(StateResolveError::ArenaUnavailable)?;
        let mut state_pointer = NonNull::from(state);
        // SAFETY: RuntimeHeap is pinned, and close has exclusive Runtime access
        // with no active execution guard. The preflight validated this exact
        // external allocation and slot generation before any shutdown work.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let state_report =
            // SAFETY: state_pointer names the still-owned main-state Box.
            unsafe { state_pointer.as_mut() }.prepare_for_runtime_shutdown(&heap.gc);
        heap.state_arena.detach_external(handle, state_pointer)?;
        self.main_state_handle = None;
        drop(self.main_state.take());
        Ok(state_report)
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
        // invariants, avoid cross-thread callbacks/reclamation; field Drop
        // releases state Boxes while the collector retains its documented
        // leak-on-drop fallback.
        if let Some(state) = self.main_state.as_deref_mut() {
            state.state_handle = None;
            state.state_arena = None;
            state.gc = None;
            state.string_pool = None;
        }
        self.main_state_handle = None;
        drop(self.main_state.take());
        self.global_root.take();
        self.registry_root.take();
        self.phase = RuntimePhase::Closed;
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
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

/// Scoped mutable access to the three execution components.
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
    /// Borrow all execution components at once.
    pub fn split_mut(&mut self) -> (&mut LuaState, &mut GarbageCollector, &mut StringPool) {
        (self.state, self.gc, self.string_pool)
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
        let released = unsafe { arena.as_mut() }.release(self.state_handle);
        debug_assert!(released);
        debug_assert!(*self.active_executions > 0);
        *self.active_executions = self.active_executions.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use lua_core::gc::header::GcObjectHeader;
    use lua_core::userdata::Userdata;

    use super::*;

    fn move_runtime(runtime: Runtime) -> Runtime {
        runtime
    }

    #[test]
    fn service_backpointers_remain_stable_when_runtime_moves() {
        let mut runtime = Runtime::new();
        let global_root = runtime.global_root().expect("global root is owned");
        let main_handle = runtime
            .main_state_handle
            .expect("main state handle is registered");
        let main_state_address = runtime
            .main_state
            .as_deref()
            .map(std::ptr::from_ref)
            .expect("running runtime has a main state");
        let (gc_address, string_pool_address) = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            assert_eq!(parts.active_execution_count(), 1);
            let (state, gc, string_pool) = parts.split_mut();
            assert_eq!(state.global_table, Some(global_root));
            assert_eq!(state.state_handle(), Some(main_handle));
            assert!(state.state_arena.is_some());
            assert!(gc.is_root(global_root));
            assert_eq!(state.gc, Some(std::ptr::from_mut(gc)));
            assert_eq!(state.string_pool, Some(std::ptr::from_mut(string_pool)));
            (std::ptr::from_mut(gc), std::ptr::from_mut(string_pool))
        };

        let mut runtime = move_runtime(runtime);
        let mut parts = runtime.parts_mut().expect("parts survive Runtime move");
        let (state, gc, string_pool) = parts.split_mut();
        assert_eq!(std::ptr::from_ref(state), main_state_address);
        assert_eq!(std::ptr::from_mut(gc), gc_address);
        assert_eq!(std::ptr::from_mut(string_pool), string_pool_address);
        assert_eq!(state.gc, Some(gc_address));
        assert_eq!(state.string_pool, Some(string_pool_address));
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
    fn close_is_idempotent_and_reports_zeroed_heap() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert!(runtime.global_root().is_some());

        let first = runtime.close().expect("first close succeeds");
        assert!(!first.already_closed);
        assert!(!first.heap_reclamation_deferred);
        assert_eq!(first.remaining_objects, 0);
        assert_eq!(first.remaining_roots, 0);
        assert_eq!(first.remaining_interned_strings, 0);
        assert_eq!(first.remaining_estimated_bytes, 0);
        assert_eq!(first.remaining_coroutine_states, 0);
        assert_eq!(first.remaining_collector_queue_entries, 0);
        assert!(first.destroyed_objects >= 2);
        assert!(first.lua_gc_callback_debt);
        assert!(first.io_resource_drain_debt);
        assert_eq!(runtime.phase(), RuntimePhase::Closed);
        assert!(runtime.main_state.is_none());
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
    fn close_rejects_active_execution_without_changing_phase() {
        let mut runtime = Runtime::new();
        runtime.active_executions = 1;

        assert!(matches!(
            runtime.close(),
            Err(RuntimeAccessError::ActiveExecutions { count: 1, .. })
        ));
        assert_eq!(runtime.phase(), RuntimePhase::Running);
        assert!(runtime.main_state.is_some());

        runtime.active_executions = 0;
        runtime.close().expect("close succeeds once inactive");
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

        // SAFETY: RuntimeHeap is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .remove_owned(first_handle)
            .expect("owned child can be removed");
        assert_eq!(heap.state_arena.live_owned_state_count(), 0);

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
    fn arena_releases_one_thousand_owned_states_back_to_baseline() {
        let mut arena = StateArena::new(RuntimeId::new(u64::MAX));
        let baseline = arena.live_owned_state_count();

        for _ in 0..1_000 {
            let handle = arena.insert_owned(Box::new(LuaState::new()));
            assert_eq!(arena.live_owned_state_count(), baseline + 1);
            arena
                .remove_owned(handle)
                .expect("an occupied owned slot can be released exactly once");
            assert_eq!(arena.live_owned_state_count(), baseline);
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
        assert_eq!(runtime.live_coroutine_state_count(), 0);
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena
                .validate(child_handle),
            Err(StateResolveError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn close_closes_main_and_coroutine_open_upvalues_before_heap_teardown() {
        let mut runtime = Runtime::new();
        {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (main, gc, _) = parts.split_mut();
            main.push_value(lua_core::value::Value::Number(1.0));
            main.find_or_create_upvalue(0, gc);

            let child_handle = main
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted");
            main.with_resolved_state_mut(child_handle, |child| {
                child.push_value(lua_core::value::Value::Number(2.0));
                child.find_or_create_upvalue(0, gc);
            })
            .expect("child resolves");
        }

        let report = runtime.close().expect("runtime closes");

        assert_eq!(report.closed_open_upvalues, 2);
        assert_eq!(report.rejected_open_upvalue_edges, 0);
        assert_eq!(report.open_upvalue_cycles, 0);
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
            main.open_upvalues = Some(foreign_upvalue);
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
            // SAFETY: fixed is a live Userdata header registered in gc.
            unsafe {
                (*(fixed.as_ptr() as *const GcObjectHeader)).mark_fixed();
            }
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
            assert_eq!(report.remaining_collector_queue_entries, 0);
            assert_eq!(report.drained_coroutine_states, 1);
        }
    }
}

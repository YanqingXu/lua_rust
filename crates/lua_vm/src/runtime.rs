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
//! The first coroutine-trampoline substrate additionally provides a
//! Runtime-owned turn driver. It confines one state borrow to one callback,
//! releases that guard before resolving a requested next handle, and retains
//! only owned `StateHandle`/result values between turns. Coroutine status,
//! result delivery, and VM continuation semantics remain a later integration.
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
use std::num::NonZeroU64;
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
use std::thread::{self, ThreadId};

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::state_handle::StateHandleIssuer;
pub use lua_core::state_handle::{RuntimeId, RuntimeIdExhausted, StateHandle};
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::thread::{CoroutineStatus, Thread};
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use thiserror::Error;

use crate::execute::{
    ExecResult, RuntimeError, VmExit, execute_proto, finish_deferred_native_call,
    finish_deferred_native_values, resume_after_deferred_native_call, resume_lua_thread,
    start_lua_call_at_stack,
};
use crate::native::{
    DeferredNativeCall, ResumeRequest, ResumeResponse, UpvalueAccessOperation, UpvalueAccessRequest,
};
use crate::state::lua_state::LuaStateShutdownReport;
use crate::state::{LuaState, ThreadStatus};

mod root_trace;
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
}

impl NativeActivationStack {
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
        gc.propagate_marks();
    }
}

#[derive(Clone, Debug)]
struct UpvalueTransferFrame {
    request: UpvalueAccessRequest,
    response: Option<UpvalueAccessResponse>,
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
    },
    DeliverUpvalue(UpvalueTransferFrame),
}

enum NativeTurnEvent {
    Request {
        request: Box<ResumeRequest>,
        caller_thread: Option<GcRef<Thread>>,
    },
    UpvalueAccess(Box<UpvalueAccessRequest>),
    Main(Result<ExecResult, RuntimeError>),
    Coroutine {
        thread: GcRef<Thread>,
        response: ResumeResponse,
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
    /// Nested arena resolution was attempted during a Runtime-owned turn.
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
    /// Internal StateArena bookkeeping disagreed before a destructive action.
    #[error("state arena invariant violated: {reason}")]
    ArenaInvariant {
        /// Stable diagnostic describing the rejected invariant.
        reason: &'static str,
    },
}

struct StateSlot {
    generation: u64,
    state: Option<NonNull<LuaState>>,
    owned: bool,
    borrowed: bool,
    retired: bool,
}

/// Runtime-owned generational arena for Lua coroutine states.
///
/// Slots contain stable `NonNull<LuaState>` addresses rather than references.
/// All resolution validates runtime, slot, generation, occupancy, current-state
/// identity, and exclusive-borrow state. References are created only inside a
/// higher-ranked closure and therefore cannot escape the resolution scope.
pub struct StateArena {
    handle_issuer: StateHandleIssuer,
    slots: Vec<StateSlot>,
    free_slots: Vec<usize>,
    live_owned_states: usize,
    turn_borrow: Option<StateHandle>,
    #[cfg(test)]
    turn_borrow_events: Vec<TurnBorrowEvent>,
    #[cfg(test)]
    peak_turn_borrowed_slots: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnBorrowEvent {
    Acquired(StateHandle),
    Released(StateHandle),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StateArenaDrainReport {
    drained_owned_states: usize,
    state_shutdown: LuaStateShutdownReport,
}

impl StateArena {
    fn new(handle_issuer: StateHandleIssuer) -> Self {
        Self {
            handle_issuer,
            slots: Vec::new(),
            free_slots: Vec::new(),
            live_owned_states: 0,
            turn_borrow: None,
            #[cfg(test)]
            turn_borrow_events: Vec::new(),
            #[cfg(test)]
            peak_turn_borrowed_slots: 0,
        }
    }

    fn runtime_id(&self) -> RuntimeId {
        self.handle_issuer.runtime_id()
    }

    fn reserve_slot(&mut self) -> StateHandle {
        let slot = loop {
            match self.free_slots.pop() {
                Some(slot_index)
                    if self.slots.get(slot_index).is_some_and(|slot| {
                        !slot.retired && slot.state.is_none() && !slot.owned && !slot.borrowed
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
                    self.slots.push(StateSlot {
                        generation: 1,
                        state: None,
                        owned: false,
                        borrowed: false,
                        retired: false,
                    });
                    break self.slots.len() - 1;
                }
            }
        };
        self.issue_handle(slot)
    }

    fn issue_handle(&self, slot: usize) -> StateHandle {
        let generation = NonZeroU64::new(self.slots[slot].generation)
            .expect("StateArena generations are always non-zero");
        self.handle_issuer.issue(slot, generation)
    }

    fn attach_external(&mut self, state: NonNull<LuaState>) -> StateHandle {
        let handle = self.reserve_slot();
        let slot = &mut self.slots[handle.slot()];
        slot.state = Some(state);
        slot.owned = false;
        handle
    }

    fn insert_owned(&mut self, mut state: Box<LuaState>) -> StateHandle {
        let next_live_owned_states = self
            .live_owned_states
            .checked_add(1)
            .expect("StateArena live owned-state count overflow");
        let handle = self.reserve_slot();
        let arena = NonNull::from(&mut *self);
        state.attach_runtime_state(handle, arena);
        let state =
            NonNull::new(Box::into_raw(state)).expect("Box::into_raw never returns a null pointer");

        let slot = &mut self.slots[handle.slot()];
        slot.state = Some(state);
        slot.owned = true;
        self.live_owned_states = next_live_owned_states;
        handle
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
            arena_ref.reject_active_turn()?;
            let state = arena_ref.resolve(handle, current_state)?;
            arena_ref.slots[handle.slot()].borrowed = true;
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

        #[cfg(test)]
        {
            let borrowed_slots = self.slots.iter().filter(|slot| slot.borrowed).count();
            self.peak_turn_borrowed_slots = self.peak_turn_borrowed_slots.max(borrowed_slots);
            self.turn_borrow_events
                .push(TurnBorrowEvent::Acquired(handle));
        }

        Ok(state)
    }

    fn begin_direct_borrow(
        &mut self,
        handle: StateHandle,
        expected_state: NonNull<LuaState>,
    ) -> Result<(), StateResolveError> {
        self.reject_active_turn()?;
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

        let next_live_owned_states = self
            .live_owned_states
            .checked_sub(1)
            .expect("occupied owned slot requires a positive live count");
        self.vacate_slot(slot_index);
        self.live_owned_states = next_live_owned_states;
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
    /// any mutation. Every state closes collector-validated open Upvalues
    /// before its slot generation is advanced or retired, then its Box is
    /// reconstructed and dropped while the collector heap is still alive.
    fn drain_owned(
        &mut self,
        gc: &mut GarbageCollector,
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

            let next_live_owned_states = self
                .live_owned_states
                .checked_sub(1)
                .expect("occupied owned slot requires a positive live count");
            // SAFETY: shutdown preflight proved that this occupied owned slot
            // is not borrowed. Its handle generation is still current while
            // the state closes every Upvalue that names it.
            report
                .state_shutdown
                .merge(unsafe { &mut *state.as_ptr() }.prepare_for_runtime_shutdown(gc));
            self.vacate_slot(slot_index);
            self.live_owned_states = next_live_owned_states;
            // SAFETY: each owned slot originates from exactly one Box::into_raw
            // and was made vacant before ownership is reconstructed here.
            let state = unsafe { Box::from_raw(state.as_ptr()) };
            report.drained_owned_states += 1;
            drop(state);
        }

        debug_assert_eq!(self.live_owned_states, 0);
        Ok(report)
    }

    fn validate_owned_drain(&self) -> Result<(), StateResolveError> {
        self.validate_internal_invariants()?;
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

    fn validate_internal_invariants(&self) -> Result<(), StateResolveError> {
        for &slot_index in &self.free_slots {
            if slot_index >= self.slots.len() {
                return Err(StateResolveError::ArenaInvariant {
                    reason: "free-list index is outside the slot table",
                });
            }
        }

        let mut occupied_owned = 0_usize;
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
                    || slot.owned
                    || slot.borrowed
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
                    if slot.owned {
                        occupied_owned = occupied_owned.checked_add(1).ok_or(
                            StateResolveError::ArenaInvariant {
                                reason: "occupied owned-state count overflowed",
                            },
                        )?;
                    }
                }
                None => {
                    if slot.owned || slot.borrowed || free_entries != 1 {
                        return Err(StateResolveError::ArenaInvariant {
                            reason: "vacant reusable slot has invalid flags or free-list count",
                        });
                    }
                }
            }
        }

        if occupied_owned != self.live_owned_states {
            return Err(StateResolveError::ArenaInvariant {
                reason: "live owned-state count disagrees with occupied slots",
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

        let slot = &mut self.slots[slot_index];
        debug_assert!(!slot.retired);
        slot.state = None;
        slot.owned = false;
        slot.borrowed = false;
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

    fn live_owned_state_count(&self) -> usize {
        self.live_owned_states
    }
}

impl Drop for StateArena {
    fn drop(&mut self) {
        self.turn_borrow = None;
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
        // pinned RuntimeHeap and its StateArena therefore still exist.
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
    native_activations: NativeActivationStack,
    _pin: PhantomPinned,
}

impl RuntimeHeap {
    fn new(handle_issuer: StateHandleIssuer) -> Self {
        Self {
            state_arena: StateArena::new(handle_issuer),
            gc: GarbageCollector::new(),
            string_pool: StringPool::new(),
            native_activations: NativeActivationStack::default(),
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
        let mut heap = Box::pin(RuntimeHeap::new(handle_issuer));

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

        Ok(Self {
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
        mut execute_turn: impl for<'turn> FnMut(
            StateHandle,
            &'turn mut LuaState,
            &'turn mut GarbageCollector,
            &'turn mut StringPool,
        ) -> RuntimeTurn<T>,
    ) -> Result<T, RuntimeAccessError> {
        self.check_owner()?;
        if self.phase != RuntimePhase::Running {
            return Err(RuntimeAccessError::NotRunning {
                runtime_id: self.id,
                phase: self.phase,
            });
        }
        if self.main_state.is_none() || self.main_state_handle.is_none() {
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

        // SAFETY: RuntimeHeap was pinned during construction and no API moves
        // its fields. The arena and service fields are disjoint, and state
        // references are confined to one HRTB callback at a time.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        let arena = NonNull::from(&mut heap.state_arena);
        let gc = &mut heap.gc;
        let string_pool = &mut heap.string_pool;
        let mut current = initial;

        loop {
            let outcome = StateArena::with_turn_state_mut(arena, current, |state| {
                execute_turn(current, state, gc, string_pool)
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
        if self.main_state.is_none() {
            return Err(RuntimeAccessError::MainStateUnavailable {
                runtime_id: self.id,
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

        self.active_executions = self
            .active_executions
            .checked_add(1)
            .expect("runtime active-execution counter overflow");
        let _active_execution = ActiveExecutionGuard {
            active_executions: &mut self.active_executions,
        };
        let runtime_id = self.id;

        // SAFETY: RuntimeHeap was pinned at construction. The arena, collector,
        // and activation stack are disjoint fields and remain at stable
        // addresses for the whole execution session.
        let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
        if !heap.native_activations.frames.is_empty()
            || !heap.native_activations.upvalue_transfers.is_empty()
        {
            return Err(RuntimeExecutionError::Protocol(
                "activation stack was not empty at execution entry".to_string(),
            ));
        }
        let arena = NonNull::from(&mut heap.state_arena);
        let gc = &mut heap.gc;
        let activations = &mut heap.native_activations;
        let _activation_cleanup = NativeActivationSessionGuard {
            activations: NonNull::from(&mut *activations),
            gc: NonNull::from(&mut *gc),
        };

        let mut current = initial;
        let mut action = NativeDriverAction::StartProto(proto);
        let mut pending_native_delivery = None;
        loop {
            activations.seed_roots(gc);
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
                matches!(&action, NativeDriverAction::DeliverUpvalue(_));
            let mut event = StateArena::with_turn_state_mut(arena, current, |state| {
                state
                    .with_native_request_scope(|state| {
                        let vm_result = execute_native_driver_action(state, gc, action);
                        classify_native_turn(state, gc, vm_result)
                    })
                    .map_err(|error| {
                        RuntimeError::new(format!("Runtime-native mailbox scope failed: {error:?}"))
                    })
                    .and_then(std::convert::identity)
            })
            .map_err(|source| RuntimeAccessError::StateArena { runtime_id, source })??;
            if completed_upvalue_transfer {
                activations.upvalue_transfers.pop().ok_or_else(|| {
                    RuntimeExecutionError::Protocol(
                        "Upvalue delivery lost its rooted transfer frame".to_string(),
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
                    event = NativeTurnEvent::Coroutine {
                        thread: *thread,
                        response: ResumeResponse::Error(runtime_error_value(
                            gc,
                            &RuntimeError::new("cannot resume dead coroutine"),
                        )),
                    };
                }
            }

            match event {
                NativeTurnEvent::Request {
                    request,
                    caller_thread,
                } => {
                    let request = *request;
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
                    let response =
                        StateArena::with_turn_state_mut(arena, request.owner, |owner_state| {
                            execute_upvalue_owner_access(owner_state, gc, &request)
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
                    action = NativeDriverAction::DeliverUpvalue(transfer.clone());
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
                    current = frame.caller;
                    action = NativeDriverAction::Deliver {
                        frame: frame.clone(),
                        response,
                        replay_dead_ancestor,
                    };
                }
                NativeTurnEvent::Main(result) => {
                    if !activations.frames.is_empty() || !activations.upvalue_transfers.is_empty() {
                        return Err(RuntimeExecutionError::Protocol(
                            "main state stopped while coroutine activations remain".to_string(),
                        ));
                    }
                    return result.map_err(RuntimeExecutionError::Vm);
                }
            }
        }
    }

    /// Execute a top-level Proto with Lua varargs and return owned results.
    ///
    /// Preparation and result collection occur outside the scheduler session;
    /// the middle execution always uses the same Runtime-owned trampoline as
    /// [`Runtime::execute_proto`].
    pub fn execute_proto_with_args(
        &mut self,
        proto: GcRef<Proto>,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, RuntimeExecutionError> {
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
        let state =
            self.main_state
                .as_deref_mut()
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;
        state.unwind_call_info_to(0);
        state.current_call_info_mut().reset();
        state.current_call_info_mut().varargs = args;
        state.top = 0;
        state.status = crate::state::ThreadStatus::Ok;

        let execution = self.execute_proto(proto);
        let state =
            self.main_state
                .as_deref_mut()
                .ok_or(RuntimeAccessError::MainStateUnavailable {
                    runtime_id: self.id,
                })?;
        let results = if execution.is_ok() {
            state_stack_values(state)
        } else {
            Vec::new()
        };
        state.unwind_call_info_to(0);
        state.current_call_info_mut().reset();
        state.top = 0;
        execution?;
        Ok(results)
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
        if !heap.native_activations.frames.is_empty()
            || !heap.native_activations.upvalue_transfers.is_empty()
        {
            return Err(StateResolveError::ArenaInvariant {
                reason: "native activation stack is not empty at shutdown",
            });
        }
        heap.state_arena
            .validate_external_detach(handle, state_pointer)?;
        heap.state_arena.validate_owned_drain()
    }

    fn shutdown_contents(&mut self) -> Result<RuntimeShutdownSummary, StateResolveError> {
        let arena_report = {
            // SAFETY: RuntimeHeap remains pinned and shutdown has exclusive
            // owner-thread access with no execution guard.
            let heap = unsafe { Pin::get_unchecked_mut(self.heap.as_mut()) };
            heap.state_arena.drain_owned(&mut heap.gc)?
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
            unsafe { state_pointer.as_mut() }.prepare_for_runtime_shutdown(&mut heap.gc);
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

impl Drop for Runtime {
    fn drop(&mut self) {
        // A RuntimePartsMut guard exclusively borrows the Runtime, so safe Rust
        // cannot enter Drop while an execution is active. The normal
        // owner-thread path delegates to the same deterministic teardown used
        // by explicit close.
        self.close_for_drop();
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
                resume_lua_thread(state, gc)
            } else if previous_status == CoroutineStatus::Normal {
                let deferred = normal_deferred.ok_or_else(|| {
                    RuntimeError::new(
                        "Normal coroutine re-entry has no deferred ancestor continuation",
                    )
                })?;
                finish_deferred_native_values(state, gc, &deferred, request.args)?;
                resume_after_deferred_native_call(state, gc, &deferred)
            } else {
                install_resume_args(state, &request.args);
                resume_lua_thread(state, gc)
            }
        }
        NativeDriverAction::Deliver {
            frame,
            response,
            replay_dead_ancestor,
        } => {
            let deferred = frame.request.deferred.as_ref().ok_or_else(|| {
                RuntimeError::new("Runtime-native activation lost its deferred call")
            })?;
            if replay_dead_ancestor {
                deferred.snapshot.restore(state);
                finish_deferred_native_values(state, gc, deferred, Vec::new())?;
            } else {
                finish_deferred_native_call(state, gc, deferred, frame.request.envelope, response)?;
            }
            resume_after_deferred_native_call(state, gc, deferred)
        }
        NativeDriverAction::DeliverUpvalue(frame) => {
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
            resume_after_upvalue_access(state, gc)
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
            let Some(thread) = state.current_thread else {
                return Ok(NativeTurnEvent::Main(Err(error)));
            };
            release_yield_permission(state)?;
            gc.with_mut(thread, |thread| {
                thread.set_status(CoroutineStatus::Dead);
            })
            .map_err(|validation| {
                RuntimeError::new(format!("invalid coroutine Thread: {validation}"))
            })?;
            let error = runtime_error_value(gc, &error);
            state.last_error = Some(error.clone());
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

fn runtime_error_value(gc: &mut GarbageCollector, error: &RuntimeError) -> Value {
    error.error_value().unwrap_or_else(|| {
        Value::String(gc.create(lua_core::gc_string::GcString::from_utf8_text(
            &error.message,
        )))
    })
}

fn release_yield_permission(state: &mut LuaState) -> Result<(), RuntimeError> {
    state.allow_yield = state
        .allow_yield
        .checked_sub(1)
        .ok_or_else(|| RuntimeError::new("coroutine yield permission underflow"))?;
    Ok(())
}

struct NativeActivationSessionGuard {
    activations: NonNull<NativeActivationStack>,
    gc: NonNull<GarbageCollector>,
}

impl Drop for NativeActivationSessionGuard {
    fn drop(&mut self) {
        // SAFETY: both pointers target disjoint fields of the pinned RuntimeHeap
        // and this guard drops before `Runtime::execute_proto` releases its
        // exclusive Runtime borrow.
        let activations = unsafe { self.activations.as_mut() };
        // SAFETY: see the invariant above; no other collector reference is used
        // during this cleanup loop.
        let gc = unsafe { self.gc.as_mut() };
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
        let gc = &runtime.heap.as_ref().get_ref().gc;
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
            .drive_state_turns(main, |handle, state, _, _| {
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
        let arena = &runtime.heap.as_ref().get_ref().state_arena;
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
    fn turn_driver_restores_state_and_active_count_when_callback_panics() {
        let mut runtime = Runtime::new();
        let main = runtime
            .main_state_handle
            .expect("main state handle is registered");

        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = runtime
                .drive_state_turns::<()>(main, |_, _, _, _| panic!("intentional turn panic"));
        }));
        assert!(panic_result.is_err());
        assert_eq!(runtime.active_execution_count(), 0);
        {
            let arena = &runtime.heap.as_ref().get_ref().state_arena;
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
            .drive_state_turns(main, |handle, _, _, _| {
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
            runtime.drive_state_turns(foreign_handle, |_, _, _, _| { RuntimeTurn::Complete(()) }),
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
        // SAFETY: RuntimeHeap is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .remove_owned(stale)
            .expect("test child can be removed");
        assert!(matches!(
            runtime.drive_state_turns(stale, |_, _, _, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::StaleGeneration { .. },
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);

        let main_ptr = NonNull::from(
            runtime
                .main_state
                .as_deref_mut()
                .expect("main state remains available"),
        );
        // SAFETY: RuntimeHeap is pinned and the test releases this synthetic
        // direct borrow before any teardown or further state access.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .begin_direct_borrow(main, main_ptr)
            .expect("test can mark the main slot borrowed");
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _, _, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::StateArena {
                source: StateResolveError::AlreadyBorrowed { .. },
                ..
            })
        ));
        assert_eq!(runtime.active_execution_count(), 0);
        // SAFETY: the same pinned arena owns the synthetic borrow above.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        assert!(heap.state_arena.release(main));
        assert!(heap.state_arena.slots.iter().all(|slot| !slot.borrowed));
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
            runtime.drive_state_turns(main, |_, _, _, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::WrongThread { .. })
        ));
        assert_eq!(runtime.active_execution_count(), 0);

        runtime.owner_thread = owner;
        runtime.active_executions = 1;
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _, _, _| RuntimeTurn::Complete(())),
            Err(RuntimeAccessError::ActiveExecutions { count: 1, .. })
        ));
        assert_eq!(runtime.active_execution_count(), 1);
        runtime.active_executions = 0;

        runtime.close().expect("runtime closes");
        assert!(matches!(
            runtime.drive_state_turns(main, |_, _, _, _| RuntimeTurn::Complete(())),
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
    fn close_preflight_rejects_free_list_corruption_before_mutation() {
        let mut runtime = Runtime::new();
        let removed = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };
        // SAFETY: RuntimeHeap is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        heap.state_arena
            .remove_owned(removed)
            .expect("child is removed");
        heap.state_arena.free_slots.push(removed.slot());

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
        assert!(runtime.main_state.is_some());
        assert!(runtime.global_root.is_some());

        // Repair the deliberately injected duplicate and prove normal shutdown
        // remains available after the fail-closed preflight.
        // SAFETY: RuntimeHeap remains pinned and the failed close did not
        // create an execution guard or mutate arena ownership.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        assert_eq!(heap.state_arena.free_slots.pop(), Some(removed.slot()));
        runtime
            .close()
            .expect("close succeeds after test corruption is repaired");
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
    fn generation_max_is_issued_once_then_the_slot_is_permanently_retired() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer);
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
        assert_eq!(arena.live_owned_state_count(), 1);
        arena
            .remove_owned(replacement)
            .expect("replacement state can be removed");
        assert_eq!(arena.live_owned_state_count(), 0);
    }

    #[test]
    fn duplicate_or_occupied_free_list_entries_cannot_overwrite_a_live_slot() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer);
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
        assert_eq!(arena.live_owned_state_count(), 2);
        assert!(arena.validate(recycled).is_ok());
        assert!(arena.validate(other).is_ok());

        arena
            .remove_owned(recycled)
            .expect("recycled state can be removed");
        arena
            .remove_owned(other)
            .expect("other state can be removed");
        assert_eq!(arena.live_owned_state_count(), 0);
    }

    #[test]
    fn arena_releases_one_thousand_owned_states_back_to_baseline() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer);
        let baseline = arena.live_owned_state_count();
        let mut first_handle = None;
        let mut previous_generation = 0;

        for _ in 0..1_000 {
            let handle = arena.insert_owned(Box::new(LuaState::new()));
            first_handle.get_or_insert(handle);
            assert!(handle.generation() > previous_generation);
            previous_generation = handle.generation();
            assert_eq!(arena.live_owned_state_count(), baseline + 1);
            arena
                .remove_owned(handle)
                .expect("an occupied owned slot can be released exactly once");
            assert_eq!(arena.live_owned_state_count(), baseline);
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
    fn arena_drain_closes_open_upvalue_before_invalidating_owner_handle() {
        let issuer =
            StateHandleIssuer::try_new().expect("test runtime namespace should be available");
        let mut arena = StateArena::new(issuer);
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

        assert_eq!(report.drained_owned_states, 1);
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
    fn close_retires_a_coroutine_at_generation_max_without_reuse_or_wrap() {
        let mut runtime = Runtime::new();
        let child_handle = {
            let mut parts = runtime.parts_mut().expect("parts are available");
            let (state, _, _) = parts.split_mut();
            state
                .insert_coroutine_state(LuaState::new())
                .expect("child is inserted")
        };

        // SAFETY: RuntimeHeap is pinned and no execution guard is live.
        let heap = unsafe { Pin::get_unchecked_mut(runtime.heap.as_mut()) };
        let arena = &mut heap.state_arena;
        let slot_index = child_handle.slot();
        arena.slots[slot_index].generation = u64::MAX;
        let final_handle = arena.issue_handle(slot_index);
        let arena_ptr = NonNull::from(&mut *arena);
        let mut child_state = arena.slots[slot_index]
            .state
            .expect("child slot remains occupied");
        // SAFETY: the slot owns this live Box and the test has exclusive
        // Runtime access; this only synchronizes its diagnostic handle with
        // the forced boundary generation.
        unsafe {
            child_state
                .as_mut()
                .attach_runtime_state(final_handle, arena_ptr);
        }

        let first = runtime.close().expect("runtime closes at generation max");
        assert_eq!(first.drained_coroutine_states, 1);
        let slot = &runtime.heap.as_ref().get_ref().state_arena.slots[slot_index];
        assert!(slot.retired);
        assert_eq!(slot.generation, u64::MAX);
        assert!(slot.state.is_none());
        assert!(
            !runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena
                .free_slots
                .contains(&slot_index)
        );
        assert!(matches!(
            runtime
                .heap
                .as_ref()
                .get_ref()
                .state_arena
                .validate(final_handle),
            Err(StateResolveError::RetiredSlot { .. })
        ));

        let second = runtime.close().expect("second close is idempotent");
        assert!(second.already_closed);
        assert_eq!(
            runtime.heap.as_ref().get_ref().state_arena.slots[slot_index].generation,
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

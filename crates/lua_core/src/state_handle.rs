//! Stable, generational identifiers for runtime-owned Lua states.
//!
//! A `StateHandle` is data, not a pointer. The owning runtime must validate
//! all three components before resolving it, so stale and cross-runtime
//! handles fail without dereferencing freed memory.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

/// Next process-unique runtime identity.
///
/// Zero is permanently invalid and `u64::MAX` is the exhausted sentinel.
/// The allocator therefore issues `1..=u64::MAX - 1`.
static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

/// Process-unique diagnostic identity of a Lua runtime.
///
/// The numeric representation is intentionally opaque: safe code can receive
/// an identity from a [`StateHandleIssuer`], but cannot reconstruct an existing
/// runtime identity from an integer.
///
/// ```compile_fail
/// use lua_core::state_handle::RuntimeId;
///
/// let forged = RuntimeId::new(7);
/// # let _ = forged;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeId(u64);

impl RuntimeId {
    /// Return the numeric diagnostic identifier.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The process-wide runtime identity namespace has been exhausted.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("process-wide RuntimeId namespace is exhausted")]
pub struct RuntimeIdExhausted;

/// Exclusive capability for issuing handles in one fresh runtime namespace.
///
/// The capability is neither `Clone` nor `Copy`. A runtime owner moves it into
/// its state arena and does not expose it again. Other safe callers may reserve
/// their own namespace, but cannot issue a handle that names an existing
/// runtime because they cannot choose or reconstruct its [`RuntimeId`].
///
/// ```compile_fail
/// use lua_core::state_handle::StateHandleIssuer;
///
/// let issuer = StateHandleIssuer::try_new().unwrap();
/// let duplicate = issuer.clone();
/// # let _ = duplicate;
/// ```
#[derive(Debug)]
pub struct StateHandleIssuer {
    runtime_id: RuntimeId,
}

impl StateHandleIssuer {
    /// Reserve a fresh process-unique runtime namespace.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeIdExhausted`] after the monotonic namespace reaches
    /// `u64::MAX`. Exhaustion is permanent and never wraps to zero.
    pub fn try_new() -> Result<Self, RuntimeIdExhausted> {
        Ok(Self {
            runtime_id: allocate_runtime_id(&NEXT_RUNTIME_ID)?,
        })
    }

    /// Return the identity owned by this issuance capability.
    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    /// Issue one value handle in this capability's namespace.
    ///
    /// The arena remains responsible for proving that `slot` is occupied and
    /// `generation` is its current generation before publishing the handle.
    /// Requiring a non-zero generation keeps the safe representation valid.
    pub fn issue(&self, slot: usize, generation: NonZeroU64) -> StateHandle {
        StateHandle {
            runtime_id: self.runtime_id,
            slot,
            generation: generation.get(),
        }
    }
}

/// Generational handle for one `LuaState` slot.
///
/// Safe code cannot construct a handle from raw identity components. Only a
/// fresh, non-duplicable [`StateHandleIssuer`] can create values in its own
/// namespace.
///
/// ```compile_fail
/// use lua_core::state_handle::{RuntimeId, StateHandle};
///
/// let forged = StateHandle::new(RuntimeId::new(7), 3, 11);
/// # let _ = forged;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateHandle {
    runtime_id: RuntimeId,
    slot: usize,
    generation: u64,
}

impl StateHandle {
    /// Return the runtime identity encoded in this handle.
    pub const fn runtime_id(self) -> RuntimeId {
        self.runtime_id
    }

    /// Return the arena slot encoded in this handle.
    pub const fn slot(self) -> usize {
        self.slot
    }

    /// Return the slot generation encoded in this handle.
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

fn allocate_runtime_id(counter: &AtomicU64) -> Result<RuntimeId, RuntimeIdExhausted> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            if current == 0 {
                None
            } else {
                current.checked_add(1)
            }
        })
        .map(RuntimeId)
        .map_err(|_| RuntimeIdExhausted)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::thread;

    use super::*;

    #[test]
    fn handle_components_are_stable_value_data() {
        let issuer = StateHandleIssuer::try_new().expect("runtime namespace is available");
        let runtime_id = issuer.runtime_id();
        let handle = issuer.issue(3, NonZeroU64::new(11).expect("generation is non-zero"));

        assert_eq!(handle.runtime_id(), runtime_id);
        assert_eq!(handle.slot(), 3);
        assert_eq!(handle.generation(), 11);
        assert_eq!(handle, handle);
    }

    #[test]
    fn generation_and_runtime_participate_in_identity() {
        let first_issuer =
            StateHandleIssuer::try_new().expect("first runtime namespace is available");
        let second_issuer =
            StateHandleIssuer::try_new().expect("second runtime namespace is available");
        let generation_one = NonZeroU64::new(1).expect("generation is non-zero");
        let generation_two = NonZeroU64::new(2).expect("generation is non-zero");
        let handle = first_issuer.issue(0, generation_one);

        assert_ne!(handle, first_issuer.issue(0, generation_two));
        assert_ne!(handle, second_issuer.issue(0, generation_one));
        assert_ne!(first_issuer.runtime_id(), second_issuer.runtime_id());
    }

    #[test]
    fn runtime_id_allocator_is_monotonic_and_never_issues_zero() {
        let counter = AtomicU64::new(1);

        let first = allocate_runtime_id(&counter).expect("first id is available");
        let second = allocate_runtime_id(&counter).expect("second id is available");

        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
        assert_ne!(first.get(), 0);
        assert_ne!(second.get(), 0);
        assert_eq!(counter.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn runtime_id_allocator_exhaustion_is_permanent_and_does_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);

        let last = allocate_runtime_id(&counter).expect("last issuable id is available");
        assert_eq!(last.get(), u64::MAX - 1);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);

        assert_eq!(allocate_runtime_id(&counter), Err(RuntimeIdExhausted));
        assert_eq!(allocate_runtime_id(&counter), Err(RuntimeIdExhausted));
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);

        let invalid_counter = AtomicU64::new(0);
        assert_eq!(
            allocate_runtime_id(&invalid_counter),
            Err(RuntimeIdExhausted)
        );
        assert_eq!(invalid_counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn concurrent_runtime_id_allocation_is_unique_and_gap_free() {
        const THREADS: usize = 8;
        const IDS_PER_THREAD: usize = 512;

        let counter = AtomicU64::new(1);
        let observed = Mutex::new(Vec::with_capacity(THREADS * IDS_PER_THREAD));
        thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    let mut local = Vec::with_capacity(IDS_PER_THREAD);
                    for _ in 0..IDS_PER_THREAD {
                        local.push(
                            allocate_runtime_id(&counter)
                                .expect("test namespace is not exhausted")
                                .get(),
                        );
                    }
                    observed
                        .lock()
                        .expect("observation lock is not poisoned")
                        .extend(local);
                });
            }
        });

        let mut observed = observed
            .into_inner()
            .expect("observation lock is not poisoned");
        observed.sort_unstable();
        assert_eq!(observed.len(), THREADS * IDS_PER_THREAD);
        assert_eq!(
            observed,
            (1..=(THREADS * IDS_PER_THREAD) as u64).collect::<Vec<_>>()
        );
        assert_eq!(
            counter.load(Ordering::Relaxed),
            (THREADS * IDS_PER_THREAD) as u64 + 1
        );
    }
}

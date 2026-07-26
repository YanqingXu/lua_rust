//! Stable, generational identifiers for runtime-owned Lua states.
//!
//! A `StateHandle` is data, not a pointer. The owning runtime must validate
//! all three components before resolving it, so stale and cross-runtime
//! handles fail without dereferencing freed memory.

/// Process-unique diagnostic identity of a Lua runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeId(u64);

impl RuntimeId {
    /// Construct an identifier from the runtime's monotonic counter.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric diagnostic identifier.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Generational handle for one `LuaState` slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateHandle {
    runtime_id: RuntimeId,
    slot: usize,
    generation: u64,
}

impl StateHandle {
    /// Construct a handle. Resolution remains subject to arena validation.
    pub const fn new(runtime_id: RuntimeId, slot: usize, generation: u64) -> Self {
        Self {
            runtime_id,
            slot,
            generation,
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_components_are_stable_value_data() {
        let runtime_id = RuntimeId::new(7);
        let handle = StateHandle::new(runtime_id, 3, 11);

        assert_eq!(handle.runtime_id(), runtime_id);
        assert_eq!(handle.slot(), 3);
        assert_eq!(handle.generation(), 11);
        assert_eq!(handle, handle);
    }

    #[test]
    fn generation_and_runtime_participate_in_identity() {
        let runtime = RuntimeId::new(1);
        let handle = StateHandle::new(runtime, 0, 1);

        assert_ne!(handle, StateHandle::new(runtime, 0, 2));
        assert_ne!(handle, StateHandle::new(RuntimeId::new(2), 0, 1));
    }
}

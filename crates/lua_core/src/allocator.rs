//! Managed-allocation accounting shared by one Runtime heap.
//!
//! Rust's system allocator does not expose portable usable-allocation sizes.
//! This ledger therefore records the exact managed payload requested by the
//! Lua runtime: GC object layouts and container capacities, canonical
//! StringPool key buffers, and Runtime-owned LuaState layouts/capacities.
//! Host allocator metadata and unrelated Rust service bookkeeping are outside
//! this contract.

use std::cell::RefCell;
use std::rc::Rc;

/// Observable managed-allocation totals for one heap lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocatorStats {
    /// Managed payload bytes currently live.
    pub live_bytes: usize,
    /// Highest simultaneously live managed payload byte count.
    pub peak_bytes: usize,
    /// Cumulative positive payload charges over this heap lifetime.
    pub total_allocated_bytes: usize,
}

/// Shared identity of one heap's managed-allocation ledger.
#[derive(Clone, Debug, Default)]
pub struct AllocationLedger(Rc<RefCell<AllocatorStats>>);

impl AllocationLedger {
    /// Create one empty allocation ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a stable snapshot of the current totals.
    pub fn stats(&self) -> AllocatorStats {
        *self.0.borrow()
    }

    fn replace_component(&self, previous: usize, current: usize) {
        let mut stats = self.0.borrow_mut();
        if current >= previous {
            let growth = current - previous;
            stats.live_bytes = stats
                .live_bytes
                .checked_add(growth)
                .expect("allocator live-byte accounting overflow");
            stats.total_allocated_bytes = stats
                .total_allocated_bytes
                .checked_add(growth)
                .expect("allocator cumulative-byte accounting overflow");
            stats.peak_bytes = stats.peak_bytes.max(stats.live_bytes);
        } else {
            stats.live_bytes = stats
                .live_bytes
                .checked_sub(previous - current)
                .expect("allocator component released more bytes than remain live");
        }
    }
}

/// One independently reconciled component in an [`AllocationLedger`].
///
/// Dropping the account releases its complete current charge.
#[derive(Debug)]
pub struct AllocationAccount {
    ledger: AllocationLedger,
    live_bytes: usize,
}

impl AllocationAccount {
    /// Create an empty component attached to `ledger`.
    pub fn new(ledger: AllocationLedger) -> Self {
        Self {
            ledger,
            live_bytes: 0,
        }
    }

    /// Replace this component's live payload charge.
    pub fn set_live_bytes(&mut self, live_bytes: usize) {
        self.ledger.replace_component(self.live_bytes, live_bytes);
        self.live_bytes = live_bytes;
    }

    /// Return this component's current charge.
    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    /// Snapshot totals shared by every component in this ledger.
    pub fn stats(&self) -> AllocatorStats {
        self.ledger.stats()
    }
}

impl Drop for AllocationAccount {
    fn drop(&mut self) {
        self.ledger.replace_component(self.live_bytes, 0);
        self.live_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_track_live_peak_total_and_drop_release() {
        let ledger = AllocationLedger::new();
        let mut first = AllocationAccount::new(ledger.clone());
        let mut second = AllocationAccount::new(ledger.clone());

        first.set_live_bytes(40);
        second.set_live_bytes(10);
        first.set_live_bytes(60);
        second.set_live_bytes(4);
        assert_eq!(
            ledger.stats(),
            AllocatorStats {
                live_bytes: 64,
                peak_bytes: 70,
                total_allocated_bytes: 70,
            }
        );

        drop(first);
        assert_eq!(ledger.stats().live_bytes, 4);
        drop(second);
        assert_eq!(ledger.stats().live_bytes, 0);
        assert_eq!(ledger.stats().peak_bytes, 70);
        assert_eq!(ledger.stats().total_allocated_bytes, 70);
    }
}

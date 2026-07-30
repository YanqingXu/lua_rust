//! Managed-allocation accounting shared by one Runtime heap.
//!
//! Rust's system allocator does not expose portable usable-allocation sizes.
//! This ledger therefore records the exact managed payload requested by the
//! Lua runtime: GC object layouts and container capacities, canonical
//! StringPool key buffers, and Runtime-owned LuaState layouts/capacities.
//! Host allocator metadata and unrelated Rust service bookkeeping are outside
//! this contract.

use std::cell::RefCell;
use std::collections::TryReserveError;
use std::fmt;
use std::rc::Rc;

use thiserror::Error;

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

/// Audited logical allocation checkpoints that support deterministic failure
/// injection.
///
/// This is not a replacement for the future Lua allocator callback. It lets
/// lifecycle tests reject a managed allocation before any owner graph,
/// publication root, or arena slot is changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationSite {
    /// One collector-owned object allocation and side-table registration.
    GcObject,
    /// One canonical StringPool key buffer.
    StringPoolKey,
    /// Capacity needed to protect a not-yet-published object.
    PublicationRoot,
    /// One new StateArena slot for a LuaState.
    StateArenaSlot,
}

impl fmt::Display for AllocationSite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GcObject => "gc-object",
            Self::StringPoolKey => "string-pool-key",
            Self::PublicationRoot => "publication-root",
            Self::StateArenaSlot => "state-arena-slot",
        })
    }
}

/// Why a checked managed allocation could not proceed.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ManagedAllocationError {
    /// The deterministic one-shot test plan selected this checkpoint.
    #[error(
        "injected managed allocation failure at {site} while requesting {requested_bytes} bytes"
    )]
    Injected {
        /// Logical owner about to allocate.
        site: AllocationSite,
        /// Managed payload or owner-record bytes requested at the checkpoint.
        requested_bytes: usize,
    },
    /// A fallible Rust container reservation rejected the requested capacity.
    #[error(
        "managed allocation capacity failure at {site} while requesting {requested_bytes} bytes"
    )]
    Capacity {
        /// Logical owner whose backing container rejected capacity.
        site: AllocationSite,
        /// Managed payload or owner-record bytes requested at the checkpoint.
        requested_bytes: usize,
    },
}

impl ManagedAllocationError {
    /// Convert a backing-container capacity failure without exposing unstable
    /// allocator diagnostics in the public error contract.
    pub fn capacity(
        site: AllocationSite,
        requested_bytes: usize,
        _source: TryReserveError,
    ) -> Self {
        Self::Capacity {
            site,
            requested_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AllocationFailurePlan {
    site: AllocationSite,
    successful_matching_checkpoints: usize,
}

#[derive(Clone, Debug, Default)]
struct AllocationLedgerState {
    stats: AllocatorStats,
    failure_plan: Option<AllocationFailurePlan>,
}

/// Shared identity of one heap's managed-allocation ledger.
#[derive(Clone, Debug, Default)]
pub struct AllocationLedger(Rc<RefCell<AllocationLedgerState>>);

impl AllocationLedger {
    /// Create one empty allocation ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a stable snapshot of the current totals.
    pub fn stats(&self) -> AllocatorStats {
        self.0.borrow().stats
    }

    /// Install a one-shot deterministic failure at `site`.
    ///
    /// `successful_matching_checkpoints` matching checkpoints are allowed
    /// before the next one fails. The plan removes itself when it fires so
    /// rollback, shutdown, and an explicit retry can proceed.
    #[doc(hidden)]
    pub fn inject_failure_after(
        &self,
        site: AllocationSite,
        successful_matching_checkpoints: usize,
    ) {
        self.0.borrow_mut().failure_plan = Some(AllocationFailurePlan {
            site,
            successful_matching_checkpoints,
        });
    }

    /// Remove a deterministic allocation-failure plan without changing
    /// accounting totals.
    #[doc(hidden)]
    pub fn clear_failure_injection(&self) {
        self.0.borrow_mut().failure_plan = None;
    }

    /// Reject the selected logical allocation before its owner graph changes.
    #[doc(hidden)]
    pub fn allocation_checkpoint(
        &self,
        site: AllocationSite,
        requested_bytes: usize,
    ) -> Result<(), ManagedAllocationError> {
        let mut state = self.0.borrow_mut();
        let Some(plan) = state.failure_plan.as_mut() else {
            return Ok(());
        };
        if plan.site != site {
            return Ok(());
        }
        if plan.successful_matching_checkpoints != 0 {
            plan.successful_matching_checkpoints -= 1;
            return Ok(());
        }
        state.failure_plan = None;
        Err(ManagedAllocationError::Injected {
            site,
            requested_bytes,
        })
    }

    fn replace_component(&self, previous: usize, current: usize) {
        let mut state = self.0.borrow_mut();
        let stats = &mut state.stats;
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

    /// Apply the shared Heap failure plan before a logical allocation mutates
    /// its owner.
    #[doc(hidden)]
    pub fn allocation_checkpoint(
        &self,
        site: AllocationSite,
        requested_bytes: usize,
    ) -> Result<(), ManagedAllocationError> {
        self.ledger.allocation_checkpoint(site, requested_bytes)
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

    #[test]
    fn injected_failure_is_site_scoped_one_shot_and_does_not_change_stats() {
        let ledger = AllocationLedger::new();
        ledger.inject_failure_after(AllocationSite::GcObject, 1);

        assert!(
            ledger
                .allocation_checkpoint(AllocationSite::StringPoolKey, 5)
                .is_ok()
        );
        assert!(
            ledger
                .allocation_checkpoint(AllocationSite::GcObject, 8)
                .is_ok()
        );
        assert_eq!(
            ledger.allocation_checkpoint(AllocationSite::GcObject, 13),
            Err(ManagedAllocationError::Injected {
                site: AllocationSite::GcObject,
                requested_bytes: 13,
            })
        );
        assert!(
            ledger
                .allocation_checkpoint(AllocationSite::GcObject, 21)
                .is_ok()
        );
        assert_eq!(ledger.stats(), AllocatorStats::default());
    }
}

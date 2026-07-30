//! Unique ownership boundary for GC-managed Lua objects and interned strings.
//!
//! A [`Heap`] keeps the collector and its canonical [`StringPool`] in one
//! destruction unit. Production hosts should own a `Heap` instead of pairing
//! standalone services themselves.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::allocator::{AllocationLedger, AllocatorStats};
use crate::gc::collector::{GarbageCollector, GcDestroyAllReport};
use crate::string_pool::StringPool;

static NEXT_HEAP_ID: AtomicU64 = AtomicU64::new(1);

/// Process-unique identity for one heap lifetime.
///
/// Zero is permanently invalid and identities are never reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeapId(NonZeroU64);

impl HeapId {
    pub(crate) fn allocate() -> Self {
        let id = NEXT_HEAP_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("process-wide HeapId namespace is exhausted");
        Self(NonZeroU64::new(id).expect("HeapId allocator never issues zero"))
    }

    /// Return the opaque identity as an integer for diagnostics.
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for HeapId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HeapId")
            .field(&self.0.get())
            .finish()
    }
}

/// Runtime-owned allocation and string-interning services.
///
/// The collector is destroyed explicitly while its string pool is still
/// alive. [`Drop`] is the safety net for callers that do not invoke
/// [`Heap::destroy_all`] themselves.
pub struct Heap {
    id: HeapId,
    allocator: AllocationLedger,
    collector: GarbageCollector,
    strings: StringPool,
}

impl Heap {
    /// Create one empty heap with a process-unique identity.
    pub fn new() -> Self {
        let id = HeapId::allocate();
        let allocator = AllocationLedger::new();
        Self {
            id,
            allocator: allocator.clone(),
            collector: GarbageCollector::new_for_heap_with_allocator(id, allocator.clone()),
            strings: StringPool::new_for_heap(id, allocator),
        }
    }

    /// Return this heap's non-reused identity.
    pub fn id(&self) -> HeapId {
        self.id
    }

    /// Borrow the collector and its canonical string pool together.
    ///
    /// The closure boundary prevents either service reference from outliving
    /// the unique mutable heap borrow.
    pub fn with_parts_mut<R>(
        &mut self,
        use_parts: impl FnOnce(&mut GarbageCollector, &mut StringPool) -> R,
    ) -> R {
        use_parts(&mut self.collector, &mut self.strings)
    }

    /// Borrow both owned services for a caller-managed lexical scope.
    pub fn parts_mut(&mut self) -> (&mut GarbageCollector, &mut StringPool) {
        (&mut self.collector, &mut self.strings)
    }

    /// Borrow the collector for validated, non-destructive reads.
    pub fn collector(&self) -> &GarbageCollector {
        &self.collector
    }

    /// Borrow the canonical string pool for non-mutating lookup.
    pub fn strings(&self) -> &StringPool {
        &self.strings
    }

    /// Borrow the collector mutably inside an exclusive Heap operation.
    pub fn collector_mut(&mut self) -> &mut GarbageCollector {
        &mut self.collector
    }

    /// Borrow the canonical string pool mutably inside an exclusive Heap operation.
    pub fn strings_mut(&mut self) -> &mut StringPool {
        &mut self.strings
    }

    /// Deterministically destroy every ordinary and fixed allocation.
    pub fn destroy_all(&mut self) -> GcDestroyAllReport {
        self.collector.destroy_all(&mut self.strings)
    }

    /// Number of currently registered GC allocations.
    pub fn object_count(&self) -> usize {
        self.collector.object_count()
    }

    /// Collector-accounted live bytes.
    pub fn accounted_bytes(&self) -> usize {
        self.collector.total_memory()
    }

    /// Managed allocator payload statistics for this Heap and attached Runtime.
    pub fn allocator_stats(&self) -> AllocatorStats {
        self.allocator.stats()
    }

    /// Clone the ledger identity for a Runtime-owned StateArena component.
    #[doc(hidden)]
    pub fn allocation_ledger(&self) -> AllocationLedger {
        self.allocator.clone()
    }

    /// Number of canonical interned strings.
    pub fn interned_string_count(&self) -> usize {
        self.strings.len()
    }

    /// Whether both owned services are empty.
    pub fn is_empty(&self) -> bool {
        self.collector.object_count() == 0 && self.strings.is_empty()
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Heap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Heap")
            .field("id", &self.id)
            .field("objects", &self.object_count())
            .field("accounted_bytes", &self.accounted_bytes())
            .field("allocator", &self.allocator_stats())
            .field("interned_strings", &self.interned_string_count())
            .finish()
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        if !self.is_empty() {
            let _ = self.destroy_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::gc_string::GcString;
    use crate::table::Table;
    use crate::value::Value;

    use super::*;

    #[test]
    fn heap_owns_one_collector_and_canonical_pool() {
        let mut heap = Heap::new();
        let heap_id = heap.id();

        heap.with_parts_mut(|collector, strings| {
            assert_eq!(collector.heap_id(), heap_id);
            assert_eq!(strings.heap_id(), Some(heap_id));
            let first = strings.intern_bytes(collector, b"owned");
            let second = strings.intern_bytes(collector, b"owned");
            assert_eq!(first, second);
            collector.create(Table::new());
        });

        assert_eq!(heap.object_count(), 2);
        assert_eq!(heap.interned_string_count(), 1);
    }

    #[test]
    fn heap_drop_destroys_strings_while_pool_is_alive() {
        let mut heap = Heap::new();
        heap.with_parts_mut(|collector, strings| {
            strings.intern_bytes(collector, b"drop");
            collector.create(GcString::from_bytes(b"noncanonical"));
        });
        assert_eq!(heap.object_count(), 2);

        drop(heap);
    }

    #[test]
    fn explicit_destroy_all_leaves_both_services_empty() {
        let mut heap = Heap::new();
        heap.with_parts_mut(|collector, strings| {
            strings.intern_bytes(collector, b"fixed");
            collector.create(Table::new());
        });

        let report = heap.destroy_all();
        assert_eq!(report.destroyed_objects(), 2);
        assert!(heap.is_empty());
    }

    #[test]
    fn allocator_stats_reconcile_object_growth_string_keys_and_teardown() {
        let mut heap = Heap::new();
        let table = heap.collector_mut().create_root(Table::new());
        let accounted_before = heap.accounted_bytes();
        let allocator_before = heap.allocator_stats();

        heap.collector_mut()
            .with_mut(table, |table| {
                for index in 1..=256 {
                    table.set(&Value::Number(index as f64), &Value::Number(index as f64));
                }
            })
            .unwrap();
        let accounted_after = heap.accounted_bytes();
        let allocator_after_table = heap.allocator_stats();
        assert!(accounted_after > accounted_before);
        assert!(allocator_after_table.live_bytes > allocator_before.live_bytes);

        heap.with_parts_mut(|collector, strings| {
            strings.intern_bytes(collector, b"allocator-key");
        });
        let allocator_after_string = heap.allocator_stats();
        assert!(allocator_after_string.live_bytes > allocator_after_table.live_bytes);
        assert!(allocator_after_string.peak_bytes >= allocator_after_string.live_bytes);
        assert!(allocator_after_string.total_allocated_bytes >= allocator_after_string.peak_bytes);

        let report = heap.destroy_all();
        assert_eq!(report.destroyed_objects(), 2);
        assert_eq!(heap.accounted_bytes(), 0);
        let final_stats = heap.allocator_stats();
        assert_eq!(final_stats.live_bytes, 0);
        assert!(final_stats.peak_bytes > 0);
        assert!(final_stats.total_allocated_bytes >= final_stats.peak_bytes);
    }
}

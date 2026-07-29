//! Unique ownership boundary for GC-managed Lua objects and interned strings.
//!
//! A [`Heap`] keeps the collector and its canonical [`StringPool`] in one
//! destruction unit. Production hosts should own a `Heap` instead of pairing
//! standalone services themselves.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

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
    collector: GarbageCollector,
    strings: StringPool,
}

impl Heap {
    /// Create one empty heap with a process-unique identity.
    pub fn new() -> Self {
        let id = HeapId::allocate();
        Self {
            id,
            collector: GarbageCollector::new_for_heap(id),
            strings: StringPool::new_for_heap(id),
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
}

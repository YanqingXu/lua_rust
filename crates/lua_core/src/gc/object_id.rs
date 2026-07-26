//! Process-unique identities for collector-managed allocations.
//!
//! Pointer addresses are reusable after an allocation is destroyed, so they
//! are not sufficient to validate a copied GC handle. `ObjectId` is assigned
//! exactly once when an object is registered and is never reused.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OBJECT_ID: AtomicU64 = AtomicU64::new(1);

/// Process-unique identity of one collector-managed allocation.
///
/// The all-zero value is reserved for `GcRef::null()`. Registered objects
/// always carry a non-zero identity.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ObjectId(u64);

impl ObjectId {
    /// Identity used by a null GC handle.
    pub const NULL: Self = Self(0);

    /// Return the numeric identity for diagnostics and serialization-free
    /// bookkeeping.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Whether this is the reserved null identity.
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn allocate() -> Self {
        let value = NEXT_OBJECT_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .unwrap_or_else(|_| panic!("GC ObjectId space exhausted"));
        debug_assert!(NonZeroU64::new(value).is_some());
        Self(value)
    }

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            f.write_str("ObjectId(NULL)")
        } else {
            write!(f, "ObjectId({})", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_ids_are_nonzero_and_monotonic() {
        let first = ObjectId::allocate();
        let second = ObjectId::allocate();

        assert!(!first.is_null());
        assert!(second > first);
        assert_eq!(ObjectId::NULL.get(), 0);
    }
}

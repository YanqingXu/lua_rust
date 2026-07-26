//! Copyable identities for collector-managed objects.
//!
//! A pointer address alone cannot distinguish a live allocation from a stale
//! handle after allocator address reuse. `GcRef<T>` therefore carries the
//! process-unique `ObjectId` assigned when the object was registered.

use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
use crate::gc::object_id::ObjectId;
use crate::types::GcObjectType;

/// A typed handle to one collector-managed allocation.
///
/// This remains `Copy`, but it is no longer pointer-sized. Safe liveness
/// checks require an owning `GarbageCollector`, whose side table validates
/// both the address and `ObjectId` before any object memory is read.
pub struct GcRef<T> {
    ptr: Option<NonNull<T>>,
    object_id: ObjectId,
    _marker: PhantomData<T>,
}

impl<T> GcRef<T> {
    /// Construct a handle for a collector-registered allocation.
    ///
    /// This is intentionally crate-private: arbitrary host pointers are light
    /// userdata, not GC references.
    ///
    /// # Safety
    ///
    /// `ptr` must identify the live allocation to which `object_id` was
    /// assigned by the collector, and the allocation must have layout `T`.
    #[inline]
    pub(crate) unsafe fn from_registered(ptr: NonNull<T>, object_id: ObjectId) -> Self {
        debug_assert!(!object_id.is_null());
        Self {
            ptr: Some(ptr),
            object_id,
            _marker: PhantomData,
        }
    }

    /// Construct a null GC handle.
    #[inline]
    pub const fn null() -> Self {
        Self {
            ptr: None,
            object_id: ObjectId::NULL,
            _marker: PhantomData,
        }
    }

    /// Whether this handle is null.
    #[inline]
    pub fn is_null(self) -> bool {
        self.ptr.is_none()
    }

    /// Return the allocation's process-unique identity.
    #[inline]
    pub const fn object_id(self) -> ObjectId {
        self.object_id
    }

    /// Return the candidate object pointer without validating liveness.
    #[inline]
    pub fn as_ptr(self) -> *const T {
        self.ptr
            .map_or(std::ptr::null(), |pointer| pointer.as_ptr().cast_const())
    }

    /// Return the candidate object pointer without validating liveness.
    #[inline]
    pub fn as_nonnull(self) -> Option<NonNull<T>> {
        self.ptr
    }

    /// Borrow the candidate allocation without a collector-side liveness
    /// check.
    ///
    /// Prefer `GarbageCollector::with_ref`. This escape hatch remains only for
    /// transitional VM paths that independently prove the object is rooted
    /// and that no sweep can occur for the duration of the borrow.
    ///
    /// # Safety
    ///
    /// The caller must prove that `(ptr, object_id)` is still registered as
    /// `T` in its owning collector and cannot be destroyed during the borrow.
    #[inline]
    pub unsafe fn as_ref(&self) -> Option<&T> {
        // SAFETY: the caller establishes liveness, layout, and borrow duration.
        self.ptr.map(|pointer| unsafe { pointer.as_ref() })
    }

    #[inline]
    pub(crate) fn erase(self) -> ErasedGcRef
    where
        T: GcObject,
    {
        ErasedGcRef {
            ptr: self
                .ptr
                .map_or(std::ptr::null_mut(), |pointer| pointer.as_ptr().cast()),
            object_id: self.object_id,
            object_type: T::expected_gc_type(),
        }
    }
}

impl<T> fmt::Debug for GcRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcRef")
            .field("ptr", &format_args!("{:p}", self.as_ptr()))
            .field("object_id", &self.object_id)
            .finish()
    }
}

impl<T> fmt::Pointer for GcRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.as_ptr(), f)
    }
}

impl<T> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for GcRef<T> {}

impl<T> PartialEq for GcRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr && self.object_id == other.object_id
    }
}

impl<T> Eq for GcRef<T> {}

impl<T> std::hash::Hash for GcRef<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
        self.object_id.hash(state);
    }
}

/// Type-erased identity retained by collector queues and root storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ErasedGcRef {
    ptr: *mut GcObjectHeader,
    object_id: ObjectId,
    object_type: GcObjectType,
}

impl ErasedGcRef {
    pub(crate) const fn ptr(self) -> *mut GcObjectHeader {
        self.ptr
    }

    pub(crate) const fn object_id(self) -> ObjectId {
        self.object_id
    }

    pub(crate) const fn object_type(self) -> GcObjectType {
        self.object_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_ref_has_reserved_identity() {
        let reference: GcRef<()> = GcRef::null();
        assert!(reference.is_null());
        assert_eq!(reference.as_ptr(), std::ptr::null());
        assert_eq!(reference.object_id(), ObjectId::NULL);
    }

    #[test]
    fn allocation_identity_participates_in_equality_and_hash() {
        let mut value = 42_u8;
        let pointer = NonNull::from(&mut value);
        let first_id = ObjectId::from_raw_for_test(7);
        let second_id = ObjectId::from_raw_for_test(8);
        // SAFETY: this unit test never dereferences either synthetic handle.
        let first = unsafe { GcRef::from_registered(pointer, first_id) };
        // SAFETY: this unit test never dereferences either synthetic handle.
        let same = unsafe { GcRef::from_registered(pointer, first_id) };
        // SAFETY: this unit test never dereferences either synthetic handle.
        let reused_address = unsafe { GcRef::from_registered(pointer, second_id) };

        assert_eq!(first, same);
        assert_ne!(first, reused_address);
    }

    #[test]
    fn gc_ref_size_debt_is_explicit() {
        assert_eq!(
            std::mem::size_of::<GcRef<u8>>(),
            2 * std::mem::size_of::<usize>(),
            "M1 provenance intentionally makes GcRef two words"
        );
    }
}

//! Non-owning Lua light-userdata pointers.
//!
//! Light userdata is an arbitrary host pointer and is never owned, traced, or
//! validated by the Lua garbage collector. Keeping it separate from `GcRef`
//! prevents host pointers from bypassing GC allocation provenance.

use std::ffi::c_void;
use std::fmt;

/// One arbitrary, possibly-null host pointer stored as Lua light userdata.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct LightUserdataRef(*mut c_void);

impl LightUserdataRef {
    /// Construct light userdata from an arbitrary host pointer.
    pub const fn from_ptr(pointer: *mut c_void) -> Self {
        Self(pointer)
    }

    /// Construct null light userdata.
    pub const fn null() -> Self {
        Self(std::ptr::null_mut())
    }

    /// Return the exact host pointer.
    pub const fn as_ptr(self) -> *mut c_void {
        self.0
    }

    /// Whether the stored host pointer is null.
    pub const fn is_null(self) -> bool {
        self.0.is_null()
    }
}

impl fmt::Debug for LightUserdataRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LightUserdataRef({:p})", self.0)
    }
}

impl fmt::Pointer for LightUserdataRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_and_null_pointers_roundtrip_exactly() {
        let mut byte = 0_u8;
        let pointer = std::ptr::from_mut(&mut byte).cast::<c_void>();
        let value = LightUserdataRef::from_ptr(pointer);

        assert_eq!(value.as_ptr(), pointer);
        assert!(!value.is_null());
        assert!(LightUserdataRef::null().is_null());
        assert!(LightUserdataRef::null().as_ptr().is_null());
        assert_eq!(
            std::mem::size_of::<LightUserdataRef>(),
            std::mem::size_of::<*mut c_void>()
        );
    }
}

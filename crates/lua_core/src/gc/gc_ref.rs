//! GC 引用安全包装器
//!
//! `GcRef<T>` 是 GC 管理对象的安全引用句柄。它包装一个裸指针，
//! 对外部 safe 代码隐藏指针细节，由 GC 系统保证指针有效性。
//!

use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// GC 管理的对象的引用句柄
///
/// `GcRef<T>` 是一个轻量级的 Copy 类型（单个指针大小），
/// 表示对 GC 管理对象的引用。只要 GC 未回收该对象，
/// `GcRef<T>` 就保持有效。
///
/// # Safety
///
/// - `GcRef<T>` 必须始终指向一个有效的 GC 对象（或为 null）
/// - GC 回收对象后，所有指向该对象的 `GcRef<T>` 变为悬空
/// - `GcRef<T>` 不实现 `Send` 或 `Sync`（单线程 Lua VM）
///
// Manual Clone/Copy impls avoid requiring T: Clone / T: Copy bounds.
// GcRef is always bitwise-copyable (it's just a pointer wrapper).
pub struct GcRef<T> {
    ptr: Option<NonNull<T>>,
    _marker: PhantomData<T>,
}

impl<T> GcRef<T> {
    /// 从裸指针创建 GcRef
    ///
    /// # Safety
    /// `ptr` 必须为 null 或指向一个有效的、未被回收的 GC 对象。
    #[inline]
    pub unsafe fn from_ptr(ptr: *const T) -> Self {
        Self {
            ptr: NonNull::new(ptr as *mut T),
            _marker: PhantomData,
        }
    }

    /// 从 NonNull 创建 GcRef
    #[inline]
    pub fn from_nonnull(ptr: NonNull<T>) -> Self {
        Self {
            ptr: Some(ptr),
            _marker: PhantomData,
        }
    }

    /// 创建空引用（null）
    #[inline]
    pub fn null() -> Self {
        Self {
            ptr: None,
            _marker: PhantomData,
        }
    }

    /// 检查是否为空（null）
    #[inline]
    pub fn is_null(&self) -> bool {
        self.ptr.is_none()
    }

    /// 获取底层裸指针
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        match self.ptr {
            Some(p) => p.as_ptr(),
            None => std::ptr::null(),
        }
    }

    /// 获取 NonNull 指针（如果非空）
    #[inline]
    pub fn as_nonnull(&self) -> Option<NonNull<T>> {
        self.ptr
    }

    /// 将 GcRef 转换为引用
    ///
    /// # Safety
    /// 调用者必须保证 GC 在此期间不会回收该对象。
    /// 在 Rust 的借用规则下，只要持有该对象的不可变引用，
    /// GC 就不能运行（GC 需要 `&mut GarbageCollector`）。
    #[inline]
    pub unsafe fn as_ref(&self) -> Option<&T> {
        // SAFETY: caller guarantees the pointer is valid for the borrow duration
        self.ptr.map(|p| unsafe { &*p.as_ptr() })
    }
}

impl<T> fmt::Debug for GcRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GcRef({:p})", self.as_ptr())
    }
}

impl<T> fmt::Pointer for GcRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.as_ptr(), f)
    }
}

// Manual Clone: bitwise copy, no T: Clone bound needed
impl<T> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

// Manual Copy: always safe since GcRef is just a NonNull + PhantomData
impl<T> Copy for GcRef<T> {}

// GcRef<T> 的比较基于指针相等性。
impl<T> PartialEq for GcRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ptr() == other.as_ptr()
    }
}

impl<T> Eq for GcRef<T> {}

impl<T> std::hash::Hash for GcRef<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ptr().hash(state);
    }
}

// Safety: 已有 `from_ptr` 作为 unsafe 构造器

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_ref() {
        let r: GcRef<()> = GcRef::null();
        assert!(r.is_null());
        assert_eq!(r.as_ptr(), std::ptr::null());
    }

    #[test]
    fn test_from_ptr() {
        let x = 42u8;
        let r: GcRef<u8> = unsafe { GcRef::from_ptr(&x) };
        assert!(!r.is_null());
        assert_eq!(r.as_ptr(), &x as *const u8);
    }

    #[test]
    fn test_pointer_equality() {
        let x = 1u8;
        let y = 2u8;
        let rx: GcRef<u8> = unsafe { GcRef::from_ptr(&x) };
        let rx2: GcRef<u8> = unsafe { GcRef::from_ptr(&x) };
        let ry: GcRef<u8> = unsafe { GcRef::from_ptr(&y) };
        assert_eq!(rx, rx2);
        assert_ne!(rx, ry);
    }

    #[test]
    fn test_gc_ref_size() {
        // GcRef<T> should be exactly one pointer wide
        assert_eq!(
            std::mem::size_of::<GcRef<u8>>(),
            std::mem::size_of::<*const u8>()
        );
    }
}

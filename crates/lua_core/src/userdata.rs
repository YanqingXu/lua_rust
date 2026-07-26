//! Lua 用户数据对象 — GC 管理的对齐原始字节缓冲区
//!
//! `Userdata` 允许将任意 C/Rust 数据包装成 Lua 对象。
//! Lua 5.1 支持两种用户数据：
//! 1. **轻量用户数据** (Light Userdata): 简单的 `void*` 指针，不受 GC 管理（对应 `Value::LightUserdata`）
//! 2. **完整用户数据** (Full Userdata): GC 管理的内存块，支持元表和终结器（本模块）
//!
//! ## 核心特性
//! - **GC 管理**：完整用户数据由 GC 自动回收
//! - **元表支持**：可设置元表实现自定义行为（如 `__gc`、`__index` 等）
//! - **终结器**：可选的数据析构回调，在 GC 回收时调用
//! - **对齐保证**：缓冲区起始地址满足平台对齐要求
//!

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::table::Table;
use crate::types::GcObjectType;

// =====================================================================
// Userdata 结构体
// =====================================================================

/// Maximum alignment supported by the built-in typed Userdata payload API.
pub const USERDATA_PAYLOAD_ALIGNMENT: usize = 16;

#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct UserdataBlock {
    bytes: [u8; USERDATA_PAYLOAD_ALIGNMENT],
}

impl UserdataBlock {
    const ZERO: Self = Self {
        bytes: [0; USERDATA_PAYLOAD_ALIGNMENT],
    };
}

/// Lua 完整用户数据对象
///
/// GC 管理的字节缓冲区，允许将任意数据包装成 Lua 对象。
/// 支持元表绑定和可选的数据析构回调。
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader
/// - data: 16-byte-aligned block storage
/// - data_len: logical byte length
/// - metatable: `Option<GcRef<Table>>`
/// - data_destructor: `Option<unsafe fn(*mut u8)>`
///
/// Typed payloads whose alignment exceeds [`USERDATA_PAYLOAD_ALIGNMENT`] are
/// rejected. This keeps the raw byte view while making the supported typed
/// payload contract explicit instead of relying on `Vec<u8>` alignment.
#[repr(C)]
pub struct Userdata {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// Stable, explicitly aligned payload allocation.
    data: Vec<UserdataBlock>,

    /// Logical payload length in bytes.
    data_len: usize,

    /// 元表指针（None 表示无元表）
    metatable: Option<GcRef<Table>>,

    /// 可选的数据析构回调
    ///
    /// 在 GC 回收此对象时调用，用于释放非平凡类型持有的外部资源。
    /// 回调接收 `data.as_mut_ptr()` 作为参数。
    ///
    data_destructor: Option<unsafe fn(*mut u8)>,

    /// Whether `write_typed` installed a live value whose padding and
    /// invariants make safe raw-byte views invalid.
    typed_payload_active: bool,
}

impl Userdata {
    /// 创建指定大小的完整用户数据（零初始化）
    ///
    pub fn new(size: usize) -> Self {
        let blocks = size.div_ceil(USERDATA_PAYLOAD_ALIGNMENT);
        Self {
            header: GcObjectHeader::new(GcObjectType::Userdata),
            data: vec![UserdataBlock::ZERO; blocks],
            data_len: size,
            metatable: None,
            data_destructor: None,
            typed_payload_active: false,
        }
    }

    /// 创建包含预初始化数据的完整用户数据
    ///
    /// 当已有已初始化的字节数据时使用此方法。
    pub fn new_with_data(data: Vec<u8>) -> Self {
        let mut userdata = Self::new(data.len());
        userdata.data_mut().copy_from_slice(&data);
        userdata
    }

    // ── 数据访问 ──────────────────────────────────────────────────

    /// 获取用户数据大小（字节）
    #[inline]
    pub fn len(&self) -> usize {
        self.data_len
    }

    /// 检查用户数据是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data_len == 0
    }

    /// 获取用户数据的不可变字节切片。
    ///
    /// # Panics
    /// Panics while a value installed by `write_typed` is live, because Rust
    /// permits uninitialized padding inside `T`.
    #[inline]
    pub fn data(&self) -> &[u8] {
        assert!(
            !self.typed_payload_active,
            "raw payload view is unavailable while a typed value is live"
        );
        // SAFETY: UserdataBlock is contiguous byte storage, `data_len` never
        // exceeds its allocated byte length, and the allocation remains
        // stable for the returned shared borrow.
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.data_len) }
    }

    /// 获取用户数据的可变字节切片。
    ///
    /// # Panics
    /// Panics while a value installed by `write_typed` is live; mutating its
    /// representation could invalidate the later typed destructor.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        assert!(
            !self.typed_payload_active,
            "raw payload view is unavailable while a typed value is live"
        );
        // SAFETY: same allocation and length invariant as `data`; the
        // exclusive borrow prevents overlapping payload access.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.data_len) }
    }

    /// 获取用户数据缓冲区的裸指针（不可变）
    ///
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr().cast()
    }

    /// 获取用户数据缓冲区的裸指针（可变）
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr().cast()
    }

    /// 获取类型化的不可变引用
    ///
    /// 如果 `sizeof<T>() > self.len()` 则返回 `None`。
    ///
    /// # Safety
    /// 调用者必须保证缓冲区中包含类型 `T` 的有效表示。
    ///
    #[inline]
    pub unsafe fn data_as<T>(&self) -> Option<&T> {
        if std::mem::size_of::<T>() > self.data_len
            || std::mem::align_of::<T>() > USERDATA_PAYLOAD_ALIGNMENT
            || !(self.as_ptr() as usize).is_multiple_of(std::mem::align_of::<T>())
        {
            return None;
        }
        // SAFETY: caller guarantees the buffer contains a valid T
        unsafe { Some(&*(self.data.as_ptr() as *const T)) }
    }

    /// 获取类型化的可变引用
    ///
    /// 如果 `sizeof<T>() > self.len()` 则返回 `None`。
    ///
    /// # Safety
    /// 调用者必须保证缓冲区中包含类型 `T` 的有效表示。
    #[inline]
    pub unsafe fn data_as_mut<T>(&mut self) -> Option<&mut T> {
        if std::mem::size_of::<T>() > self.data_len
            || std::mem::align_of::<T>() > USERDATA_PAYLOAD_ALIGNMENT
            || !(self.as_ptr() as usize).is_multiple_of(std::mem::align_of::<T>())
        {
            return None;
        }
        // SAFETY: caller guarantees the buffer contains a valid T
        unsafe { Some(&mut *(self.data.as_mut_ptr() as *mut T)) }
    }

    /// 将类型化数据写入用户数据缓冲区
    ///
    /// 使用 `std::ptr::write` 进行原始写入，实现原地构造语义。
    ///
    /// # Panics
    /// 如果 `sizeof<T>() > self.len()` 则 panic。
    ///
    /// # Safety
    /// `T` must be valid to destroy in place. If an existing payload or
    /// destructor is present, or `T` exceeds the explicit alignment contract,
    /// this method panics before construction.
    ///
    pub unsafe fn write_typed<T>(&mut self, value: T) {
        assert!(
            std::mem::size_of::<T>() <= self.data_len,
            "Userdata buffer is too small for requested type"
        );
        assert!(
            std::mem::align_of::<T>() <= USERDATA_PAYLOAD_ALIGNMENT
                && (self.as_ptr() as usize).is_multiple_of(std::mem::align_of::<T>()),
            "Userdata payload alignment is insufficient for requested type"
        );
        assert!(
            self.data_destructor.is_none() && !self.typed_payload_active,
            "Userdata already contains constructed data"
        );

        // SAFETY: caller guarantees buffer is valid for T
        unsafe {
            std::ptr::write(self.as_mut_ptr().cast::<T>(), value);
        }
        self.data_destructor = Some(Self::destroy_typed::<T>);
        self.typed_payload_active = true;
    }

    /// 类型化析构回调生成器
    unsafe fn destroy_typed<T>(ptr: *mut u8) {
        // SAFETY: ptr came from Userdata buffer which held a valid T
        unsafe {
            std::ptr::drop_in_place(ptr as *mut T);
        }
    }

    // ── 元表操作 ──────────────────────────────────────────────────

    /// 获取元表
    ///
    #[inline]
    pub fn metatable(&self) -> Option<GcRef<Table>> {
        self.metatable
    }

    /// 设置元表
    ///
    pub fn set_metatable(&mut self, mt: Option<GcRef<Table>>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, mt)
        self.metatable = mt;
    }

    /// 检查是否有元表
    ///
    #[inline]
    pub fn has_metatable(&self) -> bool {
        self.metatable.is_some()
    }

    // ── 析构器管理 ────────────────────────────────────────────────

    /// 设置数据析构回调
    ///
    /// 在 GC 回收此对象或手动调用 `run_destructor()` 时执行。
    ///
    /// # Safety
    /// The callback must accept this payload's current representation, may run
    /// at most once, and must not retain or dereference the pointer after it
    /// returns.
    pub unsafe fn set_destructor(&mut self, destructor: unsafe fn(*mut u8)) {
        self.data_destructor = Some(destructor);
    }

    /// 运行数据析构回调（如果已设置且非空则执行）
    ///
    /// 执行后清除析构器以防止重复调用。
    pub fn run_destructor(&mut self) {
        if let Some(dtor) = self.data_destructor.take() {
            // SAFETY: the destructor was registered when data was constructed;
            // the pointer points to valid data within our buffer.
            unsafe {
                dtor(self.as_mut_ptr());
            }
            for block in &mut self.data {
                block.bytes.fill(0);
            }
            self.typed_payload_active = false;
        }
    }
}

impl Drop for Userdata {
    fn drop(&mut self) {
        // 在 GC 释放内存前运行析构回调
        self.run_destructor();
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Userdata 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Userdata。
// mark_children 标记关联的元表。
unsafe impl GcObject for Userdata {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记 Userdata 引用的 GC 对象
    ///
    /// 仅标记元表（如果存在）。用户数据缓冲区的原始数据不包含 GC 引用。
    ///
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        if let Some(mt) = self.metatable {
            collector.mark_registered(mt);
        }
    }

    fn get_size(&self) -> usize {
        // 基础大小 + 用户数据缓冲区大小
        std::mem::size_of::<Self>() + self.data.capacity() * std::mem::size_of::<UserdataBlock>()
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Userdata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Userdata")
            .field("size", &self.data_len)
            .field("has_metatable", &self.metatable.is_some())
            .field("has_destructor", &self.data_destructor.is_some())
            .field("typed_payload_active", &self.typed_payload_active)
            .finish()
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::approx_constant)]

    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_ref::GcRef;
    use crate::string_pool::StringPool;

    // ── 创建测试 ──────────────────────────────────────────────────

    #[test]
    fn test_new_userdata() {
        let ud = Userdata::new(64);
        assert_eq!(ud.len(), 64);
        assert!(!ud.is_empty());
        assert!(!ud.has_metatable());
        assert!(ud.metatable().is_none());
    }

    #[test]
    fn test_new_empty_userdata() {
        let ud = Userdata::new(0);
        assert_eq!(ud.len(), 0);
        assert!(ud.is_empty());
    }

    #[test]
    fn test_new_with_data() {
        let data = vec![1, 2, 3, 4, 5];
        let ud = Userdata::new_with_data(data.clone());
        assert_eq!(ud.len(), 5);
        assert_eq!(ud.data(), &[1, 2, 3, 4, 5]);
    }

    // ── 数据访问 ──────────────────────────────────────────────────

    #[test]
    fn test_data_access() {
        let ud = Userdata::new(10);
        assert_eq!(ud.data().len(), 10);
        assert_eq!(ud.data()[0], 0); // 零初始化
    }

    #[test]
    fn test_data_mut() {
        let mut ud = Userdata::new(10);
        ud.data_mut()[0] = 42;
        ud.data_mut()[1] = 99;
        assert_eq!(ud.data()[0], 42);
        assert_eq!(ud.data()[1], 99);
    }

    #[test]
    fn test_as_ptr() {
        let mut ud = Userdata::new(10);
        ud.data_mut()[0] = 7;
        let ptr = ud.as_ptr();
        // SAFETY: the ten-byte buffer is non-empty, `ptr` points to its first
        // initialized byte, and `ud` remains immovable for this dereference.
        unsafe {
            assert_eq!(*ptr, 7);
        }
    }

    #[test]
    fn test_as_mut_ptr() {
        let mut ud = Userdata::new(10);
        let ptr = ud.as_mut_ptr();
        // SAFETY: the ten-byte buffer is non-empty and exclusively borrowed
        // through `ud`, so its first byte is valid for this write.
        unsafe {
            *ptr = 88;
        }
        assert_eq!(ud.data()[0], 88);
    }

    // ── 类型化数据操作 ────────────────────────────────────────────

    #[test]
    fn test_data_as() {
        let mut ud = Userdata::new(std::mem::size_of::<i32>());
        ud.data_mut()[0..4].copy_from_slice(&42_i32.to_le_bytes());

        // SAFETY: the buffer is large and aligned as guaranteed by `Userdata`,
        // and the copied bytes form the valid `i32` value 42.
        unsafe {
            let val: &i32 = ud.data_as::<i32>().unwrap();
            assert_eq!(*val, 42);
        }
    }

    #[test]
    fn test_data_as_too_small() {
        let ud = Userdata::new(1);
        // SAFETY: the method checks the buffer length before constructing a
        // typed reference, so this undersized case returns `None`.
        unsafe {
            assert!(ud.data_as::<i64>().is_none());
        }
    }

    #[test]
    fn test_data_as_mut() {
        let mut ud = Userdata::new(std::mem::size_of::<f64>());
        // SAFETY: `Userdata` guarantees suitable alignment, the buffer has
        // exactly enough space, and zero bytes are a valid initial `f64`.
        unsafe {
            let val: &mut f64 = ud.data_as_mut::<f64>().unwrap();
            *val = 3.14;
        }
        // SAFETY: the preceding write initialized a valid `f64` in the same
        // sufficiently sized and aligned buffer.
        unsafe {
            let val: &f64 = ud.data_as::<f64>().unwrap();
            assert!((*val - 3.14).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn payload_storage_is_stably_aligned() {
        for size in [0, 1, 15, 16, 17, 4096] {
            let userdata = Userdata::new(size);
            assert_eq!(userdata.as_ptr() as usize % USERDATA_PAYLOAD_ALIGNMENT, 0);
        }
    }

    #[test]
    fn typed_access_rejects_alignment_above_the_storage_contract() {
        #[repr(align(32))]
        struct OverAligned;

        let mut userdata = Userdata::new(std::mem::size_of::<OverAligned>());
        // SAFETY: this deliberately asks the checked typed view for an
        // unsupported alignment; it returns None before creating a reference.
        assert!(unsafe { userdata.data_as::<OverAligned>() }.is_none());
        // SAFETY: same fail-closed alignment check for the mutable view.
        assert!(unsafe { userdata.data_as_mut::<OverAligned>() }.is_none());
    }

    #[test]
    #[should_panic(expected = "payload alignment is insufficient")]
    fn write_typed_rejects_alignment_above_the_storage_contract() {
        #[repr(align(32))]
        struct OverAligned;

        let mut userdata = Userdata::new(std::mem::size_of::<OverAligned>());
        // SAFETY: this deliberately exercises the hard alignment check, which
        // panics before constructing the over-aligned payload.
        unsafe {
            userdata.write_typed(OverAligned);
        }
    }

    #[test]
    fn test_write_typed() {
        let mut ud = Userdata::new(std::mem::size_of::<i64>());
        // SAFETY: the fresh buffer is large and aligned enough for `i64` and
        // does not yet contain a constructed value.
        unsafe {
            ud.write_typed(12345_i64);
        }
        // SAFETY: `write_typed` initialized a valid `i64` that remains owned
        // by this userdata buffer.
        unsafe {
            let val: &i64 = ud.data_as::<i64>().unwrap();
            assert_eq!(*val, 12345);
        }
    }

    #[test]
    fn safe_raw_views_are_blocked_while_typed_payload_is_live() {
        let mut userdata = Userdata::new(std::mem::size_of::<u64>());
        // SAFETY: u64 fits the supported size/alignment contract and the
        // payload has not previously been constructed.
        unsafe {
            userdata.write_typed(0xfeed_beef_u64);
        }

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = userdata.data();
            }))
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = userdata.data_mut();
            }))
            .is_err()
        );
    }

    #[test]
    fn successful_typed_destructor_restores_zeroed_raw_payload() {
        let mut userdata = Userdata::new(std::mem::size_of::<u64>());
        // SAFETY: u64 fits the supported size/alignment contract.
        unsafe {
            userdata.write_typed(u64::MAX);
        }

        userdata.run_destructor();

        assert_eq!(userdata.data(), &[0; std::mem::size_of::<u64>()]);
        assert!(userdata.data_destructor.is_none());
        assert!(!userdata.typed_payload_active);
    }

    #[test]
    fn panicking_typed_destructor_keeps_raw_views_fail_closed() {
        unsafe fn panic_destructor(_payload: *mut u8) {
            panic!("probe destructor panic");
        }

        let mut userdata = Userdata::new(std::mem::size_of::<u64>());
        // SAFETY: u64 fits the supported size/alignment contract.
        unsafe {
            userdata.write_typed(7_u64);
        }
        // SAFETY: the callback deliberately panics without accessing or
        // retaining the payload pointer.
        unsafe {
            userdata.set_destructor(panic_destructor);
        }

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                userdata.run_destructor();
            }))
            .is_err()
        );
        assert!(userdata.data_destructor.is_none());
        assert!(userdata.typed_payload_active);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = userdata.data();
            }))
            .is_err()
        );
    }

    #[test]
    fn aligned_zero_sized_typed_payload_has_a_valid_lifecycle() {
        struct ZeroSized;

        let mut userdata = Userdata::new(0);
        // SAFETY: ZeroSized has supported alignment, needs no storage, and the
        // empty Vec exposes an appropriately aligned dangling pointer.
        unsafe {
            userdata.write_typed(ZeroSized);
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = userdata.data();
            }))
            .is_err()
        );

        userdata.run_destructor();
        assert!(userdata.data().is_empty());
    }

    #[test]
    #[should_panic(expected = "buffer is too small")]
    fn test_write_typed_too_small() {
        let mut ud = Userdata::new(1);
        // SAFETY: this deliberately exercises the method's size precondition
        // check; it panics before attempting the out-of-bounds write.
        unsafe {
            ud.write_typed(42_i64);
        }
    }

    #[test]
    #[should_panic(expected = "already contains constructed data")]
    fn test_write_typed_double_construct_panics() {
        let mut ud = Userdata::new(std::mem::size_of::<i32>());
        // SAFETY: the first write uses a fresh sufficiently sized buffer; the
        // second call is expected to panic before overwriting the live value.
        unsafe {
            ud.write_typed(1_i32);
            ud.write_typed(2_i32); // 不应允许重复构造
        }
    }

    // ── 析构器 ────────────────────────────────────────────────────

    #[test]
    fn test_run_destructor() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static DROPPED: AtomicBool = AtomicBool::new(false);

        unsafe fn test_dtor(ptr: *mut u8) {
            DROPPED.store(true, Ordering::SeqCst);
            // 清理类型化数据
            // SAFETY: this destructor is registered only after an `i32` was
            // constructed at `ptr`, and userdata invokes it at most once.
            unsafe {
                std::ptr::drop_in_place(ptr as *mut i32);
            }
        }

        {
            let mut ud = Userdata::new(std::mem::size_of::<i32>());
            // SAFETY: the fresh buffer is sufficiently sized and aligned for
            // `i32` and contains no previously constructed value.
            unsafe {
                ud.write_typed(42_i32);
            }
            // SAFETY: test_dtor matches the i32 installed above and does not
            // retain the payload pointer.
            unsafe {
                ud.set_destructor(test_dtor);
            }
            assert!(!DROPPED.load(Ordering::SeqCst));
            ud.run_destructor();
            assert!(DROPPED.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn test_run_destructor_only_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe fn counting_dtor(_ptr: *mut u8) {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let mut ud = Userdata::new(16);
        // SAFETY: counting_dtor does not access or retain the payload pointer.
        unsafe {
            ud.set_destructor(counting_dtor);
        }

        ud.run_destructor();
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);

        ud.run_destructor(); // 第二次应无操作
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_drop_runs_destructor() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static DROP_CALLED: AtomicBool = AtomicBool::new(false);

        unsafe fn drop_dtor(_ptr: *mut u8) {
            DROP_CALLED.store(true, Ordering::SeqCst);
        }

        {
            let mut ud = Userdata::new(8);
            // SAFETY: drop_dtor does not access or retain the payload pointer.
            unsafe {
                ud.set_destructor(drop_dtor);
            }
            // ud goes out of scope → Drop::drop → run_destructor
        }
        assert!(DROP_CALLED.load(Ordering::SeqCst));
    }

    // ── 元表管理 ──────────────────────────────────────────────────

    #[test]
    fn test_metatable_set_get() {
        let mut gc = GarbageCollector::new();
        let mut ud = Userdata::new(32);

        assert!(!ud.has_metatable());
        assert!(ud.metatable().is_none());

        let mt = gc.create(Table::new());
        ud.set_metatable(Some(mt));

        assert!(ud.has_metatable());
        assert_eq!(ud.metatable(), Some(mt));
    }

    #[test]
    fn test_metatable_remove() {
        let mut gc = GarbageCollector::new();
        let mut ud = Userdata::new(32);

        let mt = gc.create(Table::new());
        ud.set_metatable(Some(mt));
        assert!(ud.has_metatable());

        ud.set_metatable(None);
        assert!(!ud.has_metatable());
    }

    // ── GC 类型测试 ───────────────────────────────────────────────

    #[test]
    fn test_userdata_gc_header_type() {
        let ud = Userdata::new(16);
        assert_eq!(ud.gc_header().gc_type(), GcObjectType::Userdata);
    }

    #[test]
    fn test_userdata_gc_create_and_register() {
        let mut gc = GarbageCollector::new();
        let ud = Userdata::new(64);
        let ud_ref: GcRef<Userdata> = gc.create(ud);

        assert!(!ud_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    // ── GC 标记测试 ───────────────────────────────────────────────

    #[test]
    fn test_userdata_mark_metatable() {
        let mut gc = GarbageCollector::new();

        let mt = gc.create(Table::new());
        let mut ud = Userdata::new(16);
        ud.set_metatable(Some(mt));
        let ud_ref = gc.create(ud);

        gc.reset_marks();

        // SAFETY: `ud_ref` is live and registered with `gc`, which is
        // exclusively borrowed while its metatable is marked.
        unsafe {
            let ud_ptr = ud_ref.as_ptr();
            (*ud_ptr).mark_children(&mut gc);
        }

        let mt_header = mt.as_ptr() as *mut GcObjectHeader;
        // SAFETY: `mt` remains registered with `gc`; marking changes only its
        // header and does not free or relocate the table.
        unsafe {
            assert!(!(*mt_header).is_white(), "Metatable should be marked");
        }
    }

    #[test]
    fn test_userdata_mark_no_metatable() {
        let mut gc = GarbageCollector::new();

        let ud = Userdata::new(16);
        let ud_ref = gc.create(ud);

        gc.reset_marks();

        // 无元表 — mark_children 不应 panic
        // SAFETY: `ud_ref` is a live registered userdata object and `gc` is
        // exclusively borrowed for child marking.
        unsafe {
            let ud_ptr = ud_ref.as_ptr();
            (*ud_ptr).mark_children(&mut gc);
        }
    }

    // ── GC 回收测试 ───────────────────────────────────────────────

    #[test]
    fn test_userdata_swept_when_unreachable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create(Userdata::new(32));
        assert_eq!(gc.object_count(), 1);

        gc.mark();
        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 0);
    }

    #[test]
    fn test_userdata_kept_when_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create_root(Userdata::new(32));
        assert_eq!(gc.object_count(), 1);

        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_userdata_with_metatable_gc_chain() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let mt = gc.create(Table::new());
        let mut ud = Userdata::new(16);
        ud.set_metatable(Some(mt));
        gc.create_root(ud);

        assert_eq!(gc.object_count(), 2);

        let collected = gc.collect(&mut pool);
        // Root Userdata → marks metatable → both survive
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 2);
    }

    // ── get_size ──────────────────────────────────────────────────

    #[test]
    fn test_userdata_get_size() {
        let ud = Userdata::new(100);
        let size = ud.get_size();
        assert!(size >= std::mem::size_of::<Userdata>() + 100);
    }

    #[test]
    fn test_userdata_get_size_reflects_data_len() {
        let small = Userdata::new(10);
        let large = Userdata::new(1000);
        assert!(large.get_size() > small.get_size());
    }

    // ── Debug 输出 ────────────────────────────────────────────────

    #[test]
    fn test_userdata_debug() {
        let ud = Userdata::new(32);
        let debug_str = format!("{:?}", ud);
        assert!(debug_str.contains("32"));
        assert!(debug_str.contains("Userdata"));
    }

    #[test]
    fn test_userdata_debug_with_metatable() {
        let mut gc = GarbageCollector::new();
        let mut ud = Userdata::new(16);
        let mt = gc.create(Table::new());
        ud.set_metatable(Some(mt));

        let debug_str = format!("{:?}", ud);
        assert!(debug_str.contains("has_metatable"));
    }
}

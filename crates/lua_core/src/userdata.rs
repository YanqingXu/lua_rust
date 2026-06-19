//! Lua 用户数据对象 — GC 管理的原始字节缓冲区
//!
//! `Userdata` 允许将任意 C/C++/Rust 数据包装成 Lua 对象。
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
//! C++ 参考: `lua_cpp/src/core/userdata.hpp`, `lua_cpp/src/core/userdata.cpp`

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::table::Table;
use crate::types::GcObjectType;

// =====================================================================
// Userdata 结构体
// =====================================================================

/// Lua 完整用户数据对象
///
/// GC 管理的字节缓冲区，允许将任意数据包装成 Lua 对象。
/// 支持元表绑定和可选的数据析构回调。
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - data: Vec<u8> (24 bytes)
/// - metatable: Option<GcRef<Table>> (8 bytes)
/// - data_destructor: Option<unsafe fn(*mut u8)> (8 bytes)
///   总计约 56+ bytes
///
/// C++ 对应: `Lua::Userdata`（继承 `GCObject`）
#[repr(C)]
pub struct Userdata {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// 用户数据字节缓冲区
    data: Vec<u8>,

    /// 元表指针（None 表示无元表）
    metatable: Option<GcRef<Table>>,

    /// 可选的数据析构回调
    ///
    /// 在 GC 回收此对象时调用，用于释放非平凡类型持有的外部资源。
    /// 回调接收 `data.as_mut_ptr()` 作为参数。
    ///
    /// 对应 C++ 中 `UserdataBufferDeleter` 和 `dataDestructor_` 的组合。
    data_destructor: Option<unsafe fn(*mut u8)>,
}

impl Userdata {
    /// 创建指定大小的完整用户数据（零初始化）
    ///
    /// C++ 对应: `Userdata::createFull(usize size)`
    pub fn new(size: usize) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Userdata),
            data: vec![0u8; size],
            metatable: None,
            data_destructor: None,
        }
    }

    /// 创建包含预初始化数据的完整用户数据
    ///
    /// 当已有已初始化的字节数据时使用此方法。
    pub fn new_with_data(data: Vec<u8>) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Userdata),
            data,
            metatable: None,
            data_destructor: None,
        }
    }

    // ── 数据访问 ──────────────────────────────────────────────────

    /// 获取用户数据大小（字节）
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 检查用户数据是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// 获取用户数据的不可变字节切片
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// 获取用户数据的可变字节切片
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// 获取用户数据缓冲区的裸指针（不可变）
    ///
    /// C++ 对应: `Userdata::getData() const`
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// 获取用户数据缓冲区的裸指针（可变）
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }

    /// 获取类型化的不可变引用
    ///
    /// 如果 `sizeof<T>() > self.len()` 则返回 `None`。
    ///
    /// # Safety
    /// 调用者必须保证缓冲区中包含类型 `T` 的有效表示。
    ///
    /// C++ 对应: `Userdata::getTypedData<T>() const`
    #[inline]
    pub unsafe fn data_as<T>(&self) -> Option<&T> {
        if std::mem::size_of::<T>() > self.data.len() {
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
        if std::mem::size_of::<T>() > self.data.len() {
            return None;
        }
        // SAFETY: caller guarantees the buffer contains a valid T
        unsafe { Some(&mut *(self.data.as_mut_ptr() as *mut T)) }
    }

    /// 将类型化数据写入用户数据缓冲区
    ///
    /// 使用 `std::ptr::write` 进行原始写入以对齐 C++ placement new 语义。
    ///
    /// # Panics
    /// 如果 `sizeof<T>() > self.len()` 则 panic。
    ///
    /// # Safety
    /// 如果已有析构器则 panic（防止重复构造）。
    ///
    /// C++ 对应: `Userdata::constructData<T>(Args&&... args)`
    pub unsafe fn write_typed<T>(&mut self, value: T) {
        assert!(
            std::mem::size_of::<T>() <= self.data.len(),
            "Userdata buffer is too small for requested type"
        );
        assert!(
            self.data_destructor.is_none(),
            "Userdata already contains constructed data"
        );

        // SAFETY: caller guarantees buffer is valid for T
        unsafe {
            std::ptr::write(self.data.as_mut_ptr() as *mut T, value);
        }
        self.data_destructor = Some(Self::destroy_typed::<T>);
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
    /// C++ 对应: `Userdata::getMetatable() const`
    #[inline]
    pub fn metatable(&self) -> Option<GcRef<Table>> {
        self.metatable
    }

    /// 设置元表
    ///
    /// C++ 对应: `Userdata::setMetatable(Table* mt)`
    pub fn set_metatable(&mut self, mt: Option<GcRef<Table>>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, mt)
        self.metatable = mt;
    }

    /// 检查是否有元表
    ///
    /// C++ 对应: `Userdata::hasMetatable() const`
    #[inline]
    pub fn has_metatable(&self) -> bool {
        self.metatable.is_some()
    }

    // ── 析构器管理 ────────────────────────────────────────────────

    /// 设置数据析构回调
    ///
    /// 在 GC 回收此对象或手动调用 `run_destructor()` 时执行。
    pub fn set_destructor(&mut self, destructor: unsafe fn(*mut u8)) {
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
                dtor(self.data.as_mut_ptr());
            }
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
    /// C++ 对应: `Userdata::mark(GarbageCollector& gc)`
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        if let Some(mt) = self.metatable {
            // SAFETY: mt is a valid GcRef<Table> held by this Userdata;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(mt.as_ptr() as *mut GcObjectHeader);
            }
        }
    }

    fn get_size(&self) -> usize {
        // 基础大小 + 用户数据缓冲区大小
        std::mem::size_of::<Self>() + self.data.len()
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Userdata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Userdata")
            .field("size", &self.data.len())
            .field("has_metatable", &self.metatable.is_some())
            .field("has_destructor", &self.data_destructor.is_some())
            .finish()
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
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
        unsafe {
            assert_eq!(*ptr, 7);
        }
    }

    #[test]
    fn test_as_mut_ptr() {
        let mut ud = Userdata::new(10);
        let ptr = ud.as_mut_ptr();
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

        unsafe {
            let val: &i32 = ud.data_as::<i32>().unwrap();
            assert_eq!(*val, 42);
        }
    }

    #[test]
    fn test_data_as_too_small() {
        let ud = Userdata::new(1);
        unsafe {
            assert!(ud.data_as::<i64>().is_none());
        }
    }

    #[test]
    fn test_data_as_mut() {
        let mut ud = Userdata::new(std::mem::size_of::<f64>());
        unsafe {
            let val: &mut f64 = ud.data_as_mut::<f64>().unwrap();
            *val = 3.14;
        }
        unsafe {
            let val: &f64 = ud.data_as::<f64>().unwrap();
            assert!((*val - 3.14).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_write_typed() {
        let mut ud = Userdata::new(std::mem::size_of::<i64>());
        unsafe {
            ud.write_typed(12345_i64);
        }
        unsafe {
            let val: &i64 = ud.data_as::<i64>().unwrap();
            assert_eq!(*val, 12345);
        }
    }

    #[test]
    #[should_panic(expected = "buffer is too small")]
    fn test_write_typed_too_small() {
        let mut ud = Userdata::new(1);
        unsafe {
            ud.write_typed(42_i64);
        }
    }

    #[test]
    #[should_panic(expected = "already contains constructed data")]
    fn test_write_typed_double_construct_panics() {
        let mut ud = Userdata::new(std::mem::size_of::<i32>());
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
            unsafe {
                std::ptr::drop_in_place(ptr as *mut i32);
            }
        }

        {
            let mut ud = Userdata::new(std::mem::size_of::<i32>());
            unsafe {
                ud.write_typed(42_i32);
            }
            ud.set_destructor(test_dtor);
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
        ud.set_destructor(counting_dtor);

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
            ud.set_destructor(drop_dtor);
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

        unsafe {
            let ud_ptr = ud_ref.as_ptr();
            (*ud_ptr).mark_children(&mut gc);
        }

        let mt_header = mt.as_ptr() as *mut GcObjectHeader;
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

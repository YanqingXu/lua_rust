//! 垃圾回收器核心
//!
//! `GarbageCollector` 管理所有 GC 对象的生命周期，实现三色标记-清除算法。
//! 此模块提供对象创建/注册、根集管理，和 GC 循环入口点。
//! 完整的标记/清除/终结/弱表逻辑在 Phase 1.3 各自模块中实现。
//!
//! C++ 参考: `lua_cpp/src/gc/garbage_collector.hpp`, `.cpp`

use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::gc::strategy::{GcStrategy, MarkSweepGc};
use crate::types::GcColor;

/// 垃圾回收器
///
/// 管理侵入式 GC 对象链表和根集合。
/// 当前实现为 Phase 1.2 简化版 —— 完整 GC 循环在 Phase 1.3。
pub struct GarbageCollector {
    /// 所有 GC 对象的侵入式链表头
    all_objects: *mut GcObjectHeader,

    /// 根对象集合（受保护，不被回收）
    roots: Vec<*mut GcObjectHeader>,

    /// 对象计数
    object_count: usize,

    /// 估算总内存使用量（字节）
    total_memory: usize,

    /// 当前 GC 策略（Phase 1.3 启用完整收集循环）
    #[allow(dead_code)]
    strategy: Box<dyn GcStrategy>,
}

impl GarbageCollector {
    /// 创建新的 GC 实例，使用默认标记-清除策略
    pub fn new() -> Self {
        Self {
            all_objects: std::ptr::null_mut(),
            roots: Vec::new(),
            object_count: 0,
            total_memory: 0,
            strategy: Box::new(MarkSweepGc),
        }
    }

    // ── 对象创建 ──────────────────────────────────────────────

    /// 创建并注册一个 GC 管理对象
    ///
    /// 在堆上分配 `T`，将其加入 GC 链表，返回 `GcRef<T>`。
    ///
    /// # Safety
    /// T 必须实现 `GcObject`，且其析构函数不应有副作用。
    pub fn create<T: GcObject>(&mut self, obj: T) -> GcRef<T> {
        let boxed = Box::new(obj);
        let raw: *mut T = Box::into_raw(boxed);

        // SAFETY: raw 指向刚分配的 T 实例，header 偏移为 0
        let header_ptr: *mut GcObjectHeader = raw as *mut GcObjectHeader;

        // 加入侵入式链表
        // SAFETY: header_ptr 指向刚分配的 T 实例，在 Box::into_raw 后仍然有效，
        // 且 GcObjectHeader 的 Cell 字段支持通过共享引用修改
        unsafe {
            (*header_ptr).set_next(self.all_objects);
        }
        self.all_objects = header_ptr;

        // 注册到 root（临时做法 — Phase 1.3 移除，改用更精确的根集管理）
        self.object_count += 1;
        // SAFETY: raw 指向刚分配的 T 实例，仍然有效且未释放
        self.total_memory += unsafe { (*raw).get_size() };

        // SAFETY: raw 指向刚分配、已注册的有效对象
        unsafe { GcRef::from_ptr(raw) }
    }

    /// 创建并添加到根集
    pub fn create_root<T: GcObject>(&mut self, obj: T) -> GcRef<T> {
        let gc_ref = self.create(obj);
        let header_ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
        self.add_root_ptr(header_ptr);
        gc_ref
    }

    // ── 对象注册 ──────────────────────────────────────────────

    /// 将外部创建的对象注册到 GC 链表
    ///
    /// # Safety
    /// `obj` 必须指向一个有效的、尚未注册的 GC 对象。
    pub unsafe fn register_object<T: GcObject>(&mut self, obj: *const T) {
        // SAFETY: caller guarantees obj is a valid, unregistered GC object pointer
        unsafe {
            let header_ptr = obj as *mut GcObjectHeader;

            // 设置颜色为白色
            (*header_ptr).set_color(GcColor::White);

            // 加入链表
            (*header_ptr).set_next(self.all_objects);
            self.all_objects = header_ptr;

            self.object_count += 1;
            self.total_memory += (*obj).get_size();
        }
    }

    // ── 根集管理 ──────────────────────────────────────────────

    /// 添加原始 header 指针到根集
    fn add_root_ptr(&mut self, ptr: *mut GcObjectHeader) {
        if !ptr.is_null() && !self.roots.contains(&ptr) {
            self.roots.push(ptr);
        }
    }

    /// 添加 GC 对象到根集
    pub fn add_root<T: GcObject>(&mut self, gc_ref: GcRef<T>) {
        let ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
        self.add_root_ptr(ptr);
    }

    /// 从根集移除 GC 对象
    pub fn remove_root<T: GcObject>(&mut self, gc_ref: GcRef<T>) {
        let ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
        self.roots.retain(|&r| r != ptr);
    }

    /// 检查对象是否为根
    pub fn is_root<T: GcObject>(&self, gc_ref: GcRef<T>) -> bool {
        let ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
        self.roots.contains(&ptr)
    }

    // ── GC 循环 ──────────────────────────────────────────────

    /// 执行完整的标记-清除 GC 循环
    ///
    /// 返回回收的对象数量。完整实现在 Phase 1.3 完成。
    /// 当前提供骨架实现。
    pub fn collect(&mut self) -> usize {
        // Phase 1.3: 完整标记-清除实现
        // 1. 重置所有对象为白色
        // 2. 标记根集
        // 3. 传播标记
        // 4. 清除白色对象
        0 // 暂无回收（骨架）
    }

    /// 重置所有对象为白色（标记前准备）
    pub fn reset_marks(&mut self) {
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current is a node in the intrusive linked list created
            // by create(); all nodes remain valid during GC operations
            unsafe {
                (*current).set_color(GcColor::White);
                current = (*current).next();
            }
        }
    }

    /// 标记根集中的所有对象为灰色
    pub fn mark_roots(&mut self) {
        for &root in &self.roots {
            if !root.is_null() {
                // SAFETY: root is a GC-managed object in the roots vector;
                // roots remain valid while registered
                unsafe {
                    (*root).set_color(GcColor::Gray);
                }
            }
        }
    }

    /// 标记单个 GC 对象（如果为白色则标记为灰色并加入工作队列）
    ///
    /// # Safety
    /// `obj` 必须指向有效的 GC 对象。
    pub unsafe fn mark_object(&mut self, obj: *mut GcObjectHeader) {
        // SAFETY: caller guarantees obj is a valid GC object pointer
        unsafe {
            if obj.is_null() {
                return;
            }
            if (*obj).is_white() {
                (*obj).set_color(GcColor::Gray);
            }
        }
    }

    // ── 统计和查询 ──────────────────────────────────────────

    /// 获取管理的对象总数
    pub fn object_count(&self) -> usize {
        self.object_count
    }

    /// 获取根对象数量
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// 获取估算总内存（字节）
    pub fn total_memory(&self) -> usize {
        self.total_memory
    }

    /// 遍历所有对象（用于测试和调试）
    pub fn for_each_object<F: FnMut(*mut GcObjectHeader)>(&self, mut f: F) {
        let mut current = self.all_objects;
        while !current.is_null() {
            f(current);
            // SAFETY: current is a node in the intrusive linked list;
            // all nodes remain valid during iteration
            unsafe {
                current = (*current).next();
            }
        }
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        // 清理所有 GC 对象
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current comes from the intrusive list; all nodes were
            // allocated via Box::into_raw in create(). Phase 1.3 will add
            // type-aware sweep to properly convert back to Box and drop.
            let next = unsafe { (*current).next() };
            current = next;
        }
        // FIXME Phase 1.3: 实现类型感知的清理，而非简单 leak
        // 当前泄漏所有 GC 对象以避免 drop 时类型信息丢失
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GcObjectType;

    // 测试用 GC 对象
    struct TestObject {
        header: GcObjectHeader,
        #[allow(dead_code)]
        value: i32,
    }

    impl TestObject {
        fn new(value: i32) -> Self {
            Self {
                header: GcObjectHeader::new(GcObjectType::String),
                value,
            }
        }
    }

    // SAFETY: TestObject 实现了 GcObject trait 的所有要求
    unsafe impl GcObject for TestObject {
        fn gc_header(&self) -> &GcObjectHeader {
            &self.header
        }

        fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
            &mut self.header
        }

        unsafe fn mark_children(&self, _collector: &mut GarbageCollector) {
            // TestObject 不引用其他对象
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    #[test]
    fn test_create_object() {
        let mut gc = GarbageCollector::new();
        let obj = TestObject::new(42);
        let gc_ref: GcRef<TestObject> = gc.create(obj);

        assert!(!gc_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_add_remove_root() {
        let mut gc = GarbageCollector::new();
        let obj = TestObject::new(10);
        let gc_ref = gc.create(obj);

        assert!(!gc.is_root(gc_ref));
        gc.add_root(gc_ref);
        assert!(gc.is_root(gc_ref));
        assert_eq!(gc.root_count(), 1);

        gc.remove_root(gc_ref);
        assert!(!gc.is_root(gc_ref));
        assert_eq!(gc.root_count(), 0);
    }

    #[test]
    fn test_create_root() {
        let mut gc = GarbageCollector::new();
        let gc_ref = gc.create_root(TestObject::new(99));

        assert!(gc.is_root(gc_ref));
        assert_eq!(gc.object_count(), 1);
        assert_eq!(gc.root_count(), 1);
    }

    #[test]
    fn test_reset_marks() {
        let mut gc = GarbageCollector::new();
        let gc_ref = gc.create(TestObject::new(1));

        // 设置颜色为黑色
        unsafe {
            let header_ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
            (*header_ptr).set_color(GcColor::Black);
            assert!((*header_ptr).is_black());
        }

        gc.reset_marks();
        unsafe {
            let header_ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
            assert!((*header_ptr).is_white());
        }
    }

    #[test]
    fn test_object_count_tracking() {
        let mut gc = GarbageCollector::new();
        assert_eq!(gc.object_count(), 0);

        gc.create(TestObject::new(1));
        gc.create(TestObject::new(2));
        gc.create(TestObject::new(3));

        assert_eq!(gc.object_count(), 3);
    }
}

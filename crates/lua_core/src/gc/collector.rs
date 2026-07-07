//! 垃圾回收器核心
//!
//! `GarbageCollector` 管理所有 GC 对象的生命周期，实现三色标记-清除算法。
//! Phase 1.3 补全了完整的标记传播、清扫回收、弱表清理和终结器框架。
//!
//! C++ 参考: `lua_cpp/src/gc/garbage_collector.hpp`, `.cpp`

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::gc::header::bits;
use crate::gc::strategy::{GcStrategy, MarkSweepGc};
use crate::string_pool::StringPool;
use crate::table::Table;
use crate::types::GcColor;

/// 垃圾回收器
///
/// 管理侵入式 GC 对象链表和根集合。
/// Phase 1.3 补全了完整的三色标记-清除循环。
pub struct GarbageCollector {
    /// 所有 GC 对象的侵入式链表头
    pub(crate) all_objects: *mut GcObjectHeader,

    /// 根对象集合（受保护，不被回收）
    pub(crate) roots: Vec<*mut GcObjectHeader>,

    /// 灰色对象列表（待处理的标记工作队列）
    pub(crate) gray_list: Vec<*mut GcObjectHeader>,

    /// 本轮标记中发现的弱表
    pub(crate) weak_tables: Vec<*mut GcObjectHeader>,

    /// 当前进程生命周期内是否创建/标记过弱表。
    ///
    /// 自动弱表清理会用它避免在没有弱表的程序中频繁全堆扫描。
    pub(crate) weak_table_seen: bool,

    /// 等待执行 `__gc` 终结器的 userdata（Phase 1.4+ 启用）
    pub(crate) pending_finalizers: Vec<*mut GcObjectHeader>,

    /// 本轮标记中已遍历的外部 collector 对象
    pub(crate) external_marked: Vec<*mut GcObjectHeader>,

    /// 防止终结器递归执行（Phase 1.4+ 启用）
    #[allow(dead_code)]
    pub(crate) finalizers_running: bool,

    /// 对象计数
    pub(crate) object_count: usize,

    /// 估算总内存使用量（字节）
    pub(crate) total_memory: usize,

    /// 当前 GC 策略
    #[allow(dead_code)]
    strategy: Box<dyn GcStrategy>,
}

impl GarbageCollector {
    /// 创建新的 GC 实例，使用默认标记-清除策略
    pub fn new() -> Self {
        Self {
            all_objects: std::ptr::null_mut(),
            roots: Vec::new(),
            gray_list: Vec::new(),
            weak_tables: Vec::new(),
            weak_table_seen: false,
            pending_finalizers: Vec::new(),
            external_marked: Vec::new(),
            finalizers_running: false,
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
        // SAFETY: header_ptr 指向刚分配的 T 实例，在 Box::into_raw 后仍然有效
        unsafe {
            (*header_ptr).set_next(self.all_objects);
            (*header_ptr).set_color(GcColor::White);
        }
        self.all_objects = header_ptr;

        self.object_count += 1;
        // SAFETY: raw 指向刚分配的 T 实例
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
    /// Phase 1.3: 实现了完整的 mark → sweep 流程。
    /// 弱表条目在 sweep 前清理；终结器框架保留（Userdata 未实现时为空操作）。
    ///
    /// 返回回收的对象数量。
    pub fn collect(&mut self, string_pool: &mut StringPool) -> usize {
        // 1. 标记阶段：重置标记，标记根集，传播标记
        self.mark();

        // 2. 清理弱表条目（在 sweep 删除白色对象之前执行）
        self.clear_weak_table_entries();

        // 3. 清扫阶段：回收白色对象
        let collected = self.sweep(string_pool);

        // 4. 清空本轮临时列表
        self.weak_tables.clear();

        collected
    }

    /// 清空所有对象（用于测试和关闭）
    ///
    /// 强制删除所有 GC 对象、清空根集和所有内部列表。
    pub fn clear_all(&mut self, string_pool: &mut StringPool) {
        // 清空所有列表
        self.roots.clear();
        self.gray_list.clear();
        self.weak_tables.clear();
        self.weak_table_seen = false;
        self.pending_finalizers.clear();
        self.external_marked.clear();

        // 遍历链表，删除所有非固定对象
        let mut prev: *mut GcObjectHeader = std::ptr::null_mut();
        let mut current = self.all_objects;

        while !current.is_null() {
            // SAFETY: current is a valid node in the intrusive linked list
            let (next, is_fixed) = unsafe { ((*current).next(), (*current).is_fixed()) };

            if !is_fixed {
                // 从链表中移除
                if prev.is_null() {
                    self.all_objects = next;
                } else {
                    // SAFETY: prev is a valid node
                    unsafe {
                        (*prev).set_next(next);
                    }
                }

                // SAFETY: current is being removed from the list
                self.destroy_object(current, string_pool);
            } else {
                prev = current;
            }

            current = next;
        }
    }

    /// 重置所有对象为白色（标记前准备）
    pub fn reset_marks(&mut self) {
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current is a node in the intrusive linked list
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
                // SAFETY: root is a GC-managed object in the roots vector
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
            // 如果已经是灰色或黑色，不需要重复标记
            if !(*obj).is_white() {
                return;
            }
            // 标记为灰色并加入灰度列表
            (*obj).set_color(GcColor::Gray);
            self.gray_list.push(obj);
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
            // SAFETY: current is a node in the intrusive linked list
            unsafe {
                current = (*current).next();
            }
        }
    }

    /// Register a table for weak-entry cleanup without running a full GC cycle.
    pub fn register_weak_table(&mut self, table: GcRef<Table>, weak_keys: bool, weak_values: bool) {
        if !weak_keys && !weak_values {
            return;
        }
        self.weak_table_seen = true;

        let ptr = table.as_ptr() as *mut GcObjectHeader;
        if ptr.is_null() {
            return;
        }

        // SAFETY: ptr comes from a live table GcRef.
        unsafe {
            let mut marked = (*ptr).marked() & !bits::WEAKBITS;
            if weak_keys {
                marked |= bits::WEAKKEY;
            }
            if weak_values {
                marked |= bits::WEAKVALUE;
            }
            (*ptr).set_marked(marked);
        }

        if !self.weak_tables.contains(&ptr) {
            self.weak_tables.push(ptr);
        }
    }

    /// Clear entries from tables explicitly registered as weak tables.
    pub fn clear_registered_weak_tables(&mut self) {
        self.clear_weak_table_entries();
        self.weak_tables.clear();
    }

    /// Whether weak table maintenance has ever become necessary.
    pub fn has_seen_weak_table(&self) -> bool {
        self.weak_table_seen
    }

    /// 检查对象是否会在当前 sweep 中被回收
    ///
    /// C++ 对应: `GarbageCollector::isObjectDead(GCObject* obj)`
    pub fn is_object_dead(&self, obj: *mut GcObjectHeader) -> bool {
        if obj.is_null() {
            return false;
        }
        // SAFETY: obj is a valid GC object pointer
        unsafe {
            if (*obj).is_fixed() {
                return false;
            }
            (*obj).is_white()
        }
    }

    /// 检查包含 GC 引用的 Value 中的对象是否已死
    ///
    /// 字符串永远被视为存活（字符串驻留保证了即使无其他引用也可达）。
    ///
    /// C++ 对应: `GarbageCollector::isValueDead(const Value& value)`
    pub fn is_value_dead(&self, value: &crate::value::Value) -> bool {
        match value {
            crate::value::Value::String(_) => false,
            crate::value::Value::Table(t) => self.is_object_dead(t.as_ptr() as *mut GcObjectHeader),
            crate::value::Value::Function(f) => {
                self.is_object_dead(f.as_ptr() as *mut GcObjectHeader)
            }
            crate::value::Value::Userdata(u) => {
                self.is_object_dead(u.as_ptr() as *mut GcObjectHeader)
            }
            crate::value::Value::Thread(t) => {
                self.is_object_dead(t.as_ptr() as *mut GcObjectHeader)
            }
            // Nil, Boolean, Number, LightUserdata 不是 GC 对象
            _ => false,
        }
    }

    /// 检查弱值槽位是否应被清理
    ///
    /// 字符串永远不会被清理；userdata 在 pending_finalizers 中时视为已死。
    ///
    /// C++ 对应: `GarbageCollector::isWeakValueDead(const Value& value)`
    pub fn is_weak_value_dead(&self, value: &crate::value::Value) -> bool {
        match value {
            crate::value::Value::String(_) => false,
            crate::value::Value::Userdata(u) => {
                let ptr = u.as_ptr() as *mut GcObjectHeader;
                if self.pending_finalizers.contains(&ptr) {
                    return true;
                }
                // Once a userdata finalizer has run, weak-value slots should be
                // cleared on the next GC cycle even though this compatibility
                // collector does not immediately sweep the userdata object.
                // SAFETY: ptr comes from a userdata GcRef stored in this Value;
                // weak cleanup checks it before sweep can release the header.
                if unsafe { (*ptr).is_finalized() } {
                    return true;
                }
                self.is_value_dead(value)
            }
            _ => self.is_value_dead(value),
        }
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ── GC 阶段方法（实现于子模块）──────────────────────────────

impl GarbageCollector {
    // 以下方法在 gc/mark.rs、gc/sweep.rs、gc/weak.rs 中实现，
    // 但因 Rust 的 impl 块可跨文件（同 crate），只需在同模块声明。

    // mark phase — see gc/mark.rs
    // sweep phase — see gc/sweep.rs
    // weak table — see gc/weak.rs
    // finalizer  — see gc/finalize.rs
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        // 清理所有 GC 对象
        // Note: 完整的类型感知清理需要 StringPool。
        // 在没有 StringPool 的 drop 场景，对象将泄漏（测试中应调用 clear_all）。
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current comes from the intrusive list
            let next = unsafe { (*current).next() };
            if !current.is_null() {
                // SAFETY: 从链表中摘除 next 指针，避免后续重复释放
                unsafe {
                    (*current).set_next(std::ptr::null_mut());
                }
            }
            current = next;
        }
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

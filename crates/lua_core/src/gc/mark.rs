//! GC 标记阶段实现
//!
//! 实现三色标记算法的标记传播：从根对象开始，递归标记所有可达对象。
//! 包含增量写屏障以维护三色不变式。
//!
//! C++ 参考: `lua_cpp/src/gc/gc_mark.cpp`
//!
//! # Safety conventions
//! 本模块中的函数接收原始指针并对其解引用，这是 GC 内部操作的固有模式，
//! 与 C++ 参考实现一致。调用者负责保证指针有效性。

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
use crate::gc::header::bits;
use crate::table::Table;
use crate::types::{GcColor, GcObjectType};
use crate::value::Value;

impl GarbageCollector {
    /// 执行标记阶段
    ///
    /// 1. 重置所有对象为白色（保留 FIXED 和 FINALIZED 位）
    /// 2. 清空本轮临时列表
    /// 3. 标记所有根对象为灰色
    /// 4. 传播标记
    ///
    /// C++ 对应: `GarbageCollector::mark()`
    pub fn mark(&mut self) {
        // 1. 重置所有对象为白色（保留 FIXED 和 FINALIZED）
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current is a node in the intrusive linked list
            unsafe {
                let preserved = (*current).marked() & (bits::FIXED | bits::FINALIZED);
                (*current).set_marked(preserved);
                (*current).set_color(GcColor::White);
                current = (*current).next();
            }
        }

        // 2. 清空本轮临时列表
        self.gray_list.clear();
        self.weak_tables.clear();
        self.external_marked.clear();

        // 3. 标记所有根对象为灰色
        let roots: Vec<*mut GcObjectHeader> = self.roots.clone();
        for &root in &roots {
            if !root.is_null() {
                // SAFETY: root is a valid GC-managed object
                unsafe {
                    self.mark_object(root);
                }
            }
        }

        // 4. 传播标记
        self.propagate_marks();
    }

    /// 传播标记：处理灰色列表中的所有对象
    ///
    /// 从灰色列表中取出对象，将其标记为黑色，然后调用其
    /// `mark_children()` 方法报告引用关系。
    ///
    /// C++ 对应: `GarbageCollector::propagateMarks()`
    pub fn propagate_marks(&mut self) {
        while let Some(obj) = self.gray_list.pop() {
            // SAFETY: obj is from gray_list, which only contains valid GC objects
            unsafe {
                // 标记为黑色
                (*obj).set_color(GcColor::Black);

                // 如果是表，使用弱表感知的标记路径
                if (*obj).gc_type() == GcObjectType::Table {
                    self.mark_table(obj);
                } else {
                    // 调用对象的 mark_children 方法
                    // 需要通过 trait 对象调用，但 GcObject trait 的
                    // mark_children 是 unsafe 方法。
                    // 使用类型分发：透传 header 指针。
                    self.mark_object_children(obj);
                }
            }
        }
    }

    /// 调用 GC 对象的 mark_children（非 Table 类型的通用路径）
    ///
    /// # Safety
    /// `header_ptr` 必须指向有效的 GC 对象。
    unsafe fn mark_object_children(&mut self, header_ptr: *mut GcObjectHeader) {
        // SAFETY: caller guarantees header_ptr is valid
        unsafe {
            match (*header_ptr).gc_type() {
                GcObjectType::String => {
                    // GcString 的 mark_children 为空操作
                }
                GcObjectType::Table => {
                    // Table: 在 propagate_marks 中通过 mark_table 调用
                    // 这里作为 fallback 调用标准 mark_children
                    let table_ptr = header_ptr as *const Table;
                    (*table_ptr).mark_children(self);
                }
                GcObjectType::Function => {
                    let func_ptr = header_ptr as *const crate::function::Function;
                    (*func_ptr).mark_children(self);
                }
                GcObjectType::Proto => {
                    let proto_ptr = header_ptr as *const crate::proto::Proto;
                    (*proto_ptr).mark_children(self);
                }
                GcObjectType::Upval => {
                    let upval_ptr = header_ptr as *const crate::upvalue::Upvalue;
                    (*upval_ptr).mark_children(self);
                }
                GcObjectType::Userdata => {
                    let ud_ptr = header_ptr as *const crate::userdata::Userdata;
                    (*ud_ptr).mark_children(self);
                }
                GcObjectType::Thread => {
                    let thread_ptr = header_ptr as *const crate::thread::Thread;
                    (*thread_ptr).mark_children(self);
                }
            }
        }
    }

    /// 标记表对象（含弱表检测和弱模式处理）
    ///
    /// 检查表的元表 `__mode` 字段以确定弱键/弱值模式，
    /// 并将弱表注册到 `weak_tables` 列表中。
    ///
    /// C++ 对应: `GarbageCollector::markTable(Table* table)`
    pub fn mark_table(&mut self, table_header: *mut GcObjectHeader) {
        if table_header.is_null() {
            return;
        }

        // 检测弱表模式
        let (weak_keys, weak_values) = self.detect_weak_mode(table_header);

        // 设置弱表标记位
        // SAFETY: table_header is valid
        unsafe {
            let marked = (*table_header).marked() & !bits::WEAKBITS;
            let new_marked = if weak_keys {
                marked | bits::WEAKKEY
            } else {
                marked
            };
            let new_marked = if weak_values {
                new_marked | bits::WEAKVALUE
            } else {
                new_marked
            };
            (*table_header).set_marked(new_marked);
        }

        // 如果是弱表，注册到弱表列表
        if weak_keys || weak_values {
            self.weak_tables.push(table_header);
        }

        // 标记表内容（含弱键/弱值策略）
        // SAFETY: table_header is valid
        unsafe {
            let table = &*(table_header as *const Table);
            self.mark_table_contents(table, weak_keys, weak_values);
        }
    }

    /// 检测表的弱引用模式
    ///
    /// 读取表的元表 `__mode` 字段，解析其中的 `"k"` 和 `"v"` 字符。
    /// 返回 `(weak_keys, weak_values)`。
    ///
    /// 注意：当前实现依赖 StringPool 来查找 `"__mode"` 字符串。
    /// 由于标记阶段不应修改 GC 状态，此方法需要调用方提供已驻留的
    /// `"__mode"` 字符串引用。实际使用中，GlobalState 会预驻留这些字符串。
    ///
    /// Phase 1.3 简化：由于 GlobalState 尚未实现，通过遍历哈希表
    /// 查找原始字符串 `"__mode"` 来检测弱表模式。
    fn detect_weak_mode(&self, table_header: *mut GcObjectHeader) -> (bool, bool) {
        // SAFETY: table_header is valid
        let table = unsafe { &*(table_header as *const Table) };

        // 检查是否有元表
        let metatable = match table.metatable() {
            Some(mt) => mt,
            None => return (false, false),
        };

        // SAFETY: metatable is valid
        let mt = unsafe { &*metatable.as_ptr() };

        // 在元表中查找 "__mode" 键
        // 遍历哈希表查找匹配的字符串键
        // 注：这是简化实现。完整实现需要 GlobalState 预驻留 "__mode" 字符串
        // 并通过指针快速查找。
        let mode_value = self.lookup_metamethod_by_name(mt, "__mode");

        match mode_value {
            Some(Value::String(s)) => {
                // SAFETY: s is a valid GcRef
                let mode_str = unsafe { &*s.as_ptr() }.data();
                let weak_keys = mode_str.contains('k');
                let weak_values = mode_str.contains('v');
                (weak_keys, weak_values)
            }
            _ => (false, false),
        }
    }

    /// 在表中查找指定名称的字符串键对应的值
    ///
    /// Phase 1.3 过渡方案：遍历表内容查找匹配的字符串。
    /// Phase 3 实现 GlobalState 后将改用预驻留字符串直接查找。
    fn lookup_metamethod_by_name(&self, table: &Table, name: &str) -> Option<Value> {
        // 通过 next() 迭代器遍历所有键值对查找匹配的字符串键
        let mut key = Value::Nil;
        while let Some((next_key, next_value)) = table.next(&key) {
            if let Value::String(s) = &next_key {
                // SAFETY: s is valid
                let key_data = unsafe { &*s.as_ptr() }.data();
                if key_data == name {
                    return Some(next_value);
                }
            }
            key = next_key;
        }

        None
    }

    /// 标记表内容（遵循弱键/弱值策略）
    ///
    /// C++ 对应: `Table::markContents(GarbageCollector& gc, bool weakKeys, bool weakValues)`
    fn mark_table_contents(&mut self, table: &Table, weak_keys: bool, weak_values: bool) {
        // 单次遍历所有键值对
        let mut key = Value::Nil;
        while let Some((k, v)) = table.next(&key) {
            // 标记键（弱键模式下跳过非字符串键）
            if !weak_keys || k.is_string() {
                self.mark_value(&k);
            }
            // 标记值（弱值模式下只标记字符串值）
            if !weak_values || v.is_string() {
                self.mark_value(&v);
            }
            key = k;
        }

        // 标记元表（始终强引用）
        if let Some(mt) = table.metatable() {
            let header_ptr = mt.as_ptr() as *mut GcObjectHeader;
            // SAFETY: mt is a valid GC reference
            unsafe {
                self.mark_object(header_ptr);
            }
        }
    }

    /// 标记 Value 中包含的 GC 对象
    ///
    /// 如果 Value 包含可回收对象（String、Table、Function、
    /// Userdata、Thread），则标记该对象。
    ///
    /// C++ 对应: `GarbageCollector::markValue(const Value& value)`
    pub fn mark_value(&mut self, value: &Value) {
        match value {
            Value::String(s) => {
                let ptr = s.as_ptr() as *mut GcObjectHeader;
                // SAFETY: s is a valid GC reference; ptr is a valid GcObjectHeader
                unsafe {
                    self.mark_object(ptr);
                }
            }
            Value::Table(t) => {
                let ptr = t.as_ptr() as *mut GcObjectHeader;
                // SAFETY: t is a valid GC reference
                unsafe {
                    self.mark_object(ptr);
                }
            }
            Value::Function(f) => {
                let ptr = f.as_ptr() as *mut GcObjectHeader;
                // SAFETY: f is a valid GC reference
                unsafe {
                    self.mark_object(ptr);
                }
            }
            Value::Userdata(u) => {
                let ptr = u.as_ptr() as *mut GcObjectHeader;
                // SAFETY: u is a valid GC reference
                unsafe {
                    self.mark_object(ptr);
                }
            }
            Value::Thread(t) => {
                let ptr = t.as_ptr() as *mut GcObjectHeader;
                // SAFETY: t is a valid GC reference
                unsafe {
                    self.mark_object(ptr);
                }
            }
            // Nil, Boolean, Number, LightUserdata 不包含 GC 对象
            _ => {}
        }
    }

    // ── 写屏障 ──────────────────────────────────────────────────

    /// 增量 GC 写屏障
    ///
    /// 当黑色对象开始引用白色子对象时，立即标记该子对象并传播标记图。
    /// 防止同轮 sweep 回收新可达对象。
    ///
    /// C++ 对应: `GarbageCollector::writeBarrier(GCObject* owner, GCObject* child)`
    pub fn write_barrier(&mut self, owner: *mut GcObjectHeader, child: *mut GcObjectHeader) {
        if owner.is_null() || child.is_null() {
            return;
        }

        // SAFETY: owner and child are valid GC object pointers
        unsafe {
            // 仅当 owner 是黑色且 child 是白色时才需要屏障
            if !(*owner).is_black() || !(*child).is_white() {
                return;
            }
        }

        // SAFETY: child is valid
        unsafe {
            self.mark_object(child);
        }
        self.propagate_marks();
    }

    /// Value 版本的写屏障
    ///
    /// C++ 对应: `GarbageCollector::writeBarrier(GCObject* owner, const Value& value)`
    pub fn write_barrier_value(&mut self, owner: *mut GcObjectHeader, value: &Value) {
        match value {
            Value::String(s) => {
                self.write_barrier(owner, s.as_ptr() as *mut GcObjectHeader);
            }
            Value::Table(t) => {
                self.write_barrier(owner, t.as_ptr() as *mut GcObjectHeader);
            }
            Value::Function(f) => {
                self.write_barrier(owner, f.as_ptr() as *mut GcObjectHeader);
            }
            Value::Userdata(u) => {
                self.write_barrier(owner, u.as_ptr() as *mut GcObjectHeader);
            }
            Value::Thread(t) => {
                self.write_barrier(owner, t.as_ptr() as *mut GcObjectHeader);
            }
            _ => {} // 非 GC 对象不需要屏障
        }
    }

    /// 非 GC 根的写屏障（如 GlobalState 侧表）
    ///
    /// C++ 对应: `GarbageCollector::writeRootBarrier(GCObject* child)`
    pub fn write_root_barrier(&mut self, child: *mut GcObjectHeader) {
        if child.is_null() {
            return;
        }

        // SAFETY: child is valid
        unsafe {
            if !(*child).is_white() {
                return;
            }
        }

        // SAFETY: child is valid
        unsafe {
            self.mark_object(child);
        }
        self.propagate_marks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_object::GcObject;
    use crate::gc::header::GcObjectHeader;
    use crate::string_pool::StringPool;
    use crate::table::Table;
    use crate::types::{GcColor, GcObjectType};

    /// 测试用 GC 对象（包含对其他对象的引用）
    /// `#[repr(C)]` 确保 header 在偏移 0，可以安全转换为 `*mut GcObjectHeader`。
    #[repr(C)]
    struct TestObjectWithRef {
        header: GcObjectHeader,
        refs: Vec<*mut GcObjectHeader>,
    }

    impl TestObjectWithRef {
        fn new(refs: Vec<*mut GcObjectHeader>) -> Self {
            Self {
                // Use Thread type to avoid routing into type-specific mark/sweep logic
                // (Thread is the only remaining unimplemented GC type in Phase 1.4)
                header: GcObjectHeader::new(GcObjectType::Thread),
                refs,
            }
        }
    }

    unsafe impl GcObject for TestObjectWithRef {
        fn gc_header(&self) -> &GcObjectHeader {
            &self.header
        }

        fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
            &mut self.header
        }

        unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
            for &r in &self.refs {
                if !r.is_null() {
                    // SAFETY: refs contain valid GC object pointers
                    unsafe {
                        collector.mark_object(r);
                    }
                }
            }
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    #[test]
    fn test_mark_and_sweep_basic() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 创建两个 Table 对象：一个作为根，一个不是
        let root_obj = gc.create_root(Table::new());
        let _plain_obj = gc.create(Table::new());

        let root_header = root_obj.as_ptr() as *mut GcObjectHeader;

        assert_eq!(gc.object_count(), 2);

        // 执行 GC
        let collected = gc.collect(&mut pool);

        // 根对象存活，非根对象被回收
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 1);

        // 验证根对象仍为白色（sweep 后重置）
        unsafe {
            assert!((*root_header).is_white());
        }
    }

    #[test]
    fn test_mark_propagates_through_references() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 使用真实 Table 对象：parent（根）引用 child（Table 作为值存储在 parent 中）
        let child = gc.create(Table::new());
        let child_ref = child; // GcRef<Table>

        // 创建 parent Table 并引用 child
        let parent = gc.create_root(Table::new());
        // SAFETY: parent is valid
        unsafe {
            let p = &mut *(parent.as_ptr() as *mut Table);
            p.set(&Value::Number(1.0), &Value::Table(child_ref));
        }

        assert_eq!(gc.object_count(), 2);

        // 执行 GC：根 → parent → child（通过 parent 的 mark_children）全部存活
        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 2);
    }

    #[test]
    fn test_mark_value_method() {
        let mut gc = GarbageCollector::new();

        // 创建一个 Table 并注册到 GC
        let table_ref = gc.create(Table::new());
        let table_header = table_ref.as_ptr() as *mut GcObjectHeader;

        // 重置所有标记
        gc.reset_marks();
        unsafe {
            assert!((*table_header).is_white());
        }

        // 通过 mark_value 标记
        let table_value = Value::Table(table_ref);
        gc.mark_value(&table_value);

        // 应该变为灰色（在 gray_list 中）
        unsafe {
            assert!(!(*table_header).is_white());
        }
    }

    #[test]
    fn test_write_barrier() {
        let mut gc = GarbageCollector::new();

        let owner_ref = gc.create(TestObjectWithRef::new(Vec::new()));
        let child_ref = gc.create(TestObjectWithRef::new(Vec::new()));

        let owner = owner_ref.as_ptr() as *mut GcObjectHeader;
        let child = child_ref.as_ptr() as *mut GcObjectHeader;

        // 设置 owner 为黑色
        unsafe {
            (*owner).set_color(GcColor::Black);
        }
        // child 为白色
        unsafe {
            (*child).set_color(GcColor::White);
        }

        // 写屏障：黑色 owner 引用白色 child → 应标记 child
        gc.write_barrier(owner, child);

        // child 现在应该被标记（非白色）
        unsafe {
            assert!(!(*child).is_white());
        }
    }

    #[test]
    fn test_mark_clears_previous_marks() {
        let mut gc = GarbageCollector::new();

        let obj = gc.create(TestObjectWithRef::new(Vec::new()));
        let header = obj.as_ptr() as *mut GcObjectHeader;

        // 设置为黑色
        unsafe {
            (*header).set_color(GcColor::Black);
        }

        // 执行 mark —— 应重置为白色（非根对象）
        gc.mark();

        // 非根对象 → 白色
        unsafe {
            assert!((*header).is_white());
        }
    }

    #[test]
    fn test_collect_preserves_root_objects() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let _root = gc.create_root(Table::new());
        let _plain = gc.create(Table::new());

        assert_eq!(gc.object_count(), 2);

        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_object_count_and_memory_tracking() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let count_before = gc.object_count();

        gc.create(Table::new());
        gc.create(Table::new());
        gc.create(Table::new());

        assert_eq!(gc.object_count(), count_before + 3);

        // GC 回收（无根对象 → 全部回收）
        gc.collect(&mut pool);

        assert_eq!(gc.object_count(), count_before);
    }
}

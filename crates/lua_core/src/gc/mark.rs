//! GC 标记阶段实现
//!
//! 实现三色标记算法的标记传播：从根对象开始，递归标记所有可达对象。
//! 包含增量写屏障以维护三色不变式。
//!
//!
//! # Safety conventions
//! 本模块中的函数接收原始指针并对其解引用，这是 GC 内部操作的固有模式，

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::gc::header::bits;
use crate::state_handle::StateHandle;
use crate::table::Table;
use crate::types::{GcColor, GcObjectType};
use crate::value::Value;

/// Result of seeding the collector's explicit roots for a mark-only traversal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkRootSeedReport {
    /// Explicit and temporary root handles that still named registered
    /// objects.
    pub seeded: usize,
    /// Null, stale, foreign, or otherwise unregistered explicit or temporary
    /// root handles.
    pub rejected: usize,
    /// Successfully seeded lexical publication roots.
    pub temporary_seeded: usize,
    /// Rejected lexical publication roots.
    pub temporary_rejected: usize,
    /// Pending-finalizer queue entries successfully seeded as roots.
    pub pending_finalizers_seeded: usize,
    /// Stale, foreign, or mistyped pending-finalizer entries rejected.
    pub pending_finalizers_rejected: usize,
}

/// One concrete-object propagation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkTraceStep {
    /// Concrete object layout that was traced.
    pub object_type: GcObjectType,
    /// State edge carried by a reachable `Thread`, if present.
    pub thread_state_handle: Option<StateHandle>,
    /// Owner state edge carried by a reachable open `Upvalue`, if present.
    pub upvalue_state_handle: Option<StateHandle>,
    /// Whether the reachable `Thread` also published a caller edge.
    pub traced_thread_caller: bool,
}

impl GarbageCollector {
    /// 执行标记阶段
    ///
    /// 1. 重置所有对象为白色（保留 FIXED 和 FINALIZED 位）
    /// 2. 清空本轮临时列表
    /// 3. 标记所有根对象为灰色
    /// 4. 传播标记
    ///
    pub fn mark(&mut self) {
        self.begin_mark_only();
        self.propagate_marks();
    }

    /// Reset mark state and seed registered explicit roots without sweeping.
    ///
    /// This is the collector half of Runtime-owned live-set diagnostics. It
    /// never calls `sweep`, `collect`, `clear_all`, finalizers, or object
    /// destruction. State roots are deliberately not accepted here: the
    /// Runtime owns and validates those through its `StateArena`.
    pub fn begin_mark_only(&mut self) -> MarkRootSeedReport {
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
        self.rejected_mark_edges = 0;

        // 3. 标记所有仍属于本 collector 的显式根对象为灰色
        let mut report = MarkRootSeedReport::default();
        let roots = self.roots.clone();
        for root in roots {
            match self.validate_erased(root) {
                Ok(pointer) => {
                    self.mark_live_object(pointer.as_ptr());
                    report.seeded += 1;
                }
                Err(_) => {
                    report.rejected += 1;
                    self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                }
            }
        }

        let temporary_roots: Vec<_> = self.temporary_roots.values().copied().collect();
        for root in temporary_roots {
            match self.validate_erased(root) {
                Ok(pointer) => {
                    self.mark_live_object(pointer.as_ptr());
                    report.seeded += 1;
                    report.temporary_seeded += 1;
                }
                Err(_) => {
                    report.rejected += 1;
                    report.temporary_rejected += 1;
                    self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                }
            }
        }

        let pending_finalizers = self.pending_finalizers.clone();
        for userdata in pending_finalizers {
            match self.validate_ref(userdata) {
                Ok(pointer) => {
                    self.mark_live_object(pointer.as_ptr().cast());
                    report.seeded += 1;
                    report.pending_finalizers_seeded += 1;
                }
                Err(_) => {
                    report.rejected += 1;
                    report.pending_finalizers_rejected += 1;
                    self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                }
            }
        }

        report
    }

    /// 传播标记：处理灰色列表中的所有对象
    ///
    /// 从灰色列表中取出对象，将其标记为黑色，然后调用其
    /// `mark_children()` 方法报告引用关系。
    ///
    pub fn propagate_marks(&mut self) {
        while self.propagate_one_marked_object().is_some() {}
    }

    /// Trace one gray object and return any state edge it publishes.
    ///
    /// Runtime's canonical mark-only tracer uses this single-step API to
    /// alternate between the collector gray queue and its validated
    /// `StateHandle` queue until both reach a fixed point.
    pub fn propagate_one_marked_object(&mut self) -> Option<MarkTraceStep> {
        let obj = self.gray_list.pop()?;

        let Some(live) = self.live_allocations.get(&(obj as usize)).copied() else {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return None;
        };

        // SAFETY: gray_list contains only live headers registered through
        // `create` or validated by `mark_registered`; membership was checked
        // again before dereference.
        unsafe {
            (*obj).set_color(GcColor::Black);
            let object_type = live.object_type;
            let (thread_state_handle, traced_thread_caller) = if object_type == GcObjectType::Thread
            {
                let thread = &*(obj as *const crate::thread::Thread);
                (thread.state_handle(), thread.caller().is_some())
            } else {
                (None, false)
            };
            let upvalue_state_handle = if object_type == GcObjectType::Upval {
                let upvalue = &*(obj as *const crate::upvalue::Upvalue);
                upvalue.open_location().map(|(owner, _)| owner)
            } else {
                None
            };

            if object_type == GcObjectType::Table {
                self.mark_table(obj);
            } else {
                self.mark_object_children(obj, object_type);
            }

            Some(MarkTraceStep {
                object_type,
                thread_state_handle,
                upvalue_state_handle,
                traced_thread_caller,
            })
        }
    }

    /// 调用 GC 对象的 mark_children（非 Table 类型的通用路径）
    ///
    /// # Safety
    /// `header_ptr` 必须指向有效的 GC 对象。
    unsafe fn mark_object_children(
        &mut self,
        header_ptr: *mut GcObjectHeader,
        object_type: GcObjectType,
    ) {
        // SAFETY: caller guarantees header_ptr is valid
        unsafe {
            match object_type {
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
    fn mark_table(&mut self, table_header: *mut GcObjectHeader) {
        let Some(table_ref) = self.registered_ref_from_ptr(table_header.cast::<Table>()) else {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return;
        };
        let Ok(table_pointer) = self.validate_ref(table_ref) else {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return;
        };
        let table_header = table_pointer.as_ptr().cast::<GcObjectHeader>();

        // 检测弱表模式
        let (weak_keys, weak_values) = self.detect_weak_mode(table_header);

        // 设置弱表标记位
        // SAFETY: the side table matched pointer, ObjectId, and Table tag.
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
        if (weak_keys || weak_values) && !self.weak_tables.contains(&table_ref) {
            self.weak_tables.push(table_ref);
        }

        // 标记表内容（含弱键/弱值策略）
        // SAFETY: the validated allocation remains live while `&mut self`
        // prevents collector destruction.
        unsafe {
            let table = table_pointer.as_ref();
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

        let Ok(mt_pointer) = self.validate_ref(metatable) else {
            return (false, false);
        };
        // SAFETY: validation matched address, identity, and Table tag.
        let mt = unsafe { mt_pointer.as_ref() };

        // 在元表中查找 "__mode" 键
        // 遍历哈希表查找匹配的字符串键
        // 注：这是简化实现。完整实现需要 GlobalState 预驻留 "__mode" 字符串
        // 并通过指针快速查找。
        let mode_value = self.lookup_metamethod_by_name(mt, "__mode");

        match mode_value {
            Some(Value::String(s)) => self
                .with_string_bytes(s, |mode| (mode.contains(&b'k'), mode.contains(&b'v')))
                .unwrap_or((false, false)),
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
            if let Value::String(s) = &next_key
                && self
                    .with_string_bytes(*s, |bytes| bytes == name.as_bytes())
                    .unwrap_or(false)
            {
                return Some(next_value);
            }
            key = next_key;
        }

        None
    }

    /// 标记表内容（遵循弱键/弱值策略）
    ///
    fn mark_table_contents(&mut self, table: &Table, weak_keys: bool, weak_values: bool) {
        // 单次遍历所有键值对
        let mut key = Value::Nil;
        while let Some((k, v)) = table.next(&key) {
            // 标记键（弱键模式下跳过非字符串键）
            if !weak_keys || k.is_string() {
                self.mark_registered_value(&k);
            }
            // 标记值（弱值模式下只标记字符串值）
            if !weak_values || v.is_string() {
                self.mark_registered_value(&v);
            }
            key = k;
        }

        // 标记元表（始终强引用）
        if let Some(mt) = table.metatable() {
            self.mark_registered(mt);
        }
    }

    /// 标记 Value 中包含的 GC 对象
    ///
    /// 如果 Value 包含可回收对象（String、Table、Function、
    /// Userdata、Thread），则标记该对象。
    ///
    pub fn mark_value(&mut self, value: &Value) {
        self.mark_registered_value(value);
    }

    /// Mark a typed reference only if it belongs to this collector and its
    /// concrete tag matches `T`.
    ///
    /// The candidate pointer is compared, not dereferenced, until membership
    /// in the collector's live intrusive list has been established.
    pub fn mark_registered<T: GcObject>(&mut self, value: GcRef<T>) -> bool {
        match self.validate_ref(value) {
            Ok(pointer) => {
                self.mark_live_object(pointer.as_ptr().cast::<GcObjectHeader>());
                true
            }
            Err(_) => {
                self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                false
            }
        }
    }

    /// Checked mark operation for the collectable edge in a `Value`.
    ///
    /// Non-collectable values have no edge and therefore return `true`.
    pub fn mark_registered_value(&mut self, value: &Value) -> bool {
        match value {
            Value::String(value) => self.mark_registered(*value),
            Value::Table(value) => self.mark_registered(*value),
            Value::Function(value) => self.mark_registered(*value),
            Value::Userdata(value) => self.mark_registered(*value),
            Value::Thread(value) => self.mark_registered(*value),
            _ => true,
        }
    }

    // ── 写屏障 ──────────────────────────────────────────────────

    /// 增量 GC 写屏障
    ///
    /// 当黑色对象开始引用白色子对象时，立即标记该子对象并传播标记图。
    /// 防止同轮 sweep 回收新可达对象。
    ///
    pub fn write_barrier<O: GcObject, C: GcObject>(
        &mut self,
        owner: GcRef<O>,
        child: GcRef<C>,
    ) -> bool {
        let (Ok(owner), Ok(child)) = (self.validate_ref(owner), self.validate_ref(child)) else {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return false;
        };
        let owner = owner.as_ptr().cast::<GcObjectHeader>();
        let child = child.as_ptr().cast::<GcObjectHeader>();

        // SAFETY: both handles matched collector, allocation identity, and
        // concrete type before either header is read.
        unsafe {
            if !(*owner).is_black() || !(*child).is_white() {
                return true;
            }
        }

        self.mark_live_object(child);
        self.propagate_marks();
        true
    }

    /// Value 版本的写屏障
    ///
    pub fn write_barrier_value<O: GcObject>(&mut self, owner: GcRef<O>, value: &Value) -> bool {
        match value {
            Value::String(child) => self.write_barrier(owner, *child),
            Value::Table(child) => self.write_barrier(owner, *child),
            Value::Function(child) => self.write_barrier(owner, *child),
            Value::Userdata(child) => self.write_barrier(owner, *child),
            Value::Thread(child) => self.write_barrier(owner, *child),
            _ => self.contains_registered(owner),
        }
    }

    /// 非 GC 根的写屏障（如 GlobalState 侧表）
    ///
    pub fn write_root_barrier<C: GcObject>(&mut self, child: GcRef<C>) -> bool {
        let Ok(child) = self.validate_ref(child) else {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return false;
        };
        let child = child.as_ptr().cast::<GcObjectHeader>();
        // SAFETY: validation matched address, identity, and concrete tag.
        unsafe {
            if !(*child).is_white() {
                return true;
            }
        }

        self.mark_live_object(child);
        self.propagate_marks();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::header::GcObjectHeader;
    use crate::string_pool::StringPool;
    use crate::table::Table;
    use crate::thread::Thread;
    use crate::types::GcColor;
    use crate::userdata::Userdata;

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
        // SAFETY: `root_obj` is a registered root, so collection preserved it;
        // GC objects are not relocated.
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
        // SAFETY: `table_ref` is still owned by `gc`; resetting marks neither
        // frees nor relocates it.
        unsafe {
            assert!((*table_header).is_white());
        }

        // 通过 mark_value 标记
        let table_value = Value::Table(table_ref);
        gc.mark_value(&table_value);

        // 应该变为灰色（在 gray_list 中）
        // SAFETY: marking retains the registered table and only updates its
        // header bits.
        unsafe {
            assert!(!(*table_header).is_white());
        }
    }

    #[test]
    fn mark_only_seed_rejects_unregistered_explicit_root_without_dereference() {
        let mut gc = GarbageCollector::new();
        let live = gc.create_root(Table::new());
        // SAFETY: synthetic handle is never dereferenced; it exercises the
        // fail-closed test injection path only.
        let unregistered = unsafe {
            GcRef::<Table>::from_registered(
                std::ptr::NonNull::<Table>::dangling(),
                crate::gc::object_id::ObjectId::from_raw_for_test(u64::MAX),
            )
        };
        gc.roots.push(unregistered.erase());

        let report = gc.begin_mark_only();

        assert_eq!(report.seeded, 1);
        assert_eq!(report.rejected, 1);
        let live_header = live.as_ptr() as *const GcObjectHeader;
        // SAFETY: `live` remains registered and mark-only never destroys it.
        assert!(!unsafe { (*live_header).is_white() });
    }

    #[test]
    fn child_tracing_rejects_foreign_identity_before_header_read() {
        let mut owner_gc = GarbageCollector::new();
        let mut foreign_gc = GarbageCollector::new();
        let parent = owner_gc.create_root(Table::new());
        let foreign_child = foreign_gc.create(Table::new());

        owner_gc
            .with_mut(parent, |table| {
                table.set(&Value::Number(1.0), &Value::Table(foreign_child));
            })
            .unwrap();

        let report = owner_gc.begin_mark_only();
        owner_gc.propagate_marks();

        assert_eq!(report.seeded, 1);
        assert_eq!(owner_gc.marked_object_count(), 1);
        assert_eq!(owner_gc.rejected_mark_edge_count(), 1);
    }

    #[test]
    fn barriers_reject_foreign_and_stale_handles_fail_closed() {
        let mut gc = GarbageCollector::new();
        let mut foreign_gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let owner = gc.create(Thread::new());
        let stale_child = gc.create(Thread::new());
        let foreign_child = foreign_gc.create(Thread::new());
        let owner_header = owner.as_ptr() as *mut GcObjectHeader;

        // SAFETY: owner is registered and kept black so it survives sweep;
        // stale_child remains white and is reclaimed.
        unsafe {
            (*owner_header).set_color(GcColor::Black);
        }
        assert!(!gc.write_barrier(owner, foreign_child));
        assert!(!gc.write_root_barrier(foreign_child));

        assert_eq!(gc.sweep(&mut pool), 1);
        assert!(gc.contains_registered(owner));
        assert!(!gc.contains_registered(stale_child));
        assert!(!gc.write_barrier(owner, stale_child));
        assert!(!gc.write_root_barrier(stale_child));
    }

    #[test]
    fn two_million_registered_barrier_dispatches_remain_stable() {
        let mut gc = GarbageCollector::new();

        // Use real Thread layouts so propagation exercises the same concrete
        // dispatch that production objects use. The previous fake Thread tag
        // made this test invoke undefined behavior.
        let owner_ref = gc.create(Thread::new());
        let child_ref = gc.create(Thread::new());

        let owner = owner_ref.as_ptr() as *mut GcObjectHeader;
        let child = child_ref.as_ptr() as *mut GcObjectHeader;

        // Repeated dispatch is an in-process regression stress test for the
        // formerly intermittent misaligned-pointer failure.
        for _ in 0..2_000_000 {
            // SAFETY: both pointers are live registered Thread headers. No
            // collection or relocation occurs during this loop.
            unsafe {
                (*owner).set_color(GcColor::Black);
                (*child).set_color(GcColor::White);
            }

            // 写屏障：黑色 owner 引用白色 child → 应标记 child
            assert!(gc.write_barrier(owner_ref, child_ref));

            // SAFETY: the barrier marks and traces `child` without freeing it.
            unsafe {
                assert!((*child).is_black());
            }
        }
    }

    #[test]
    fn test_mark_clears_previous_marks() {
        let mut gc = GarbageCollector::new();

        let obj = gc.create(Thread::new());
        let header = obj.as_ptr() as *mut GcObjectHeader;

        // 设置为黑色
        // SAFETY: `header` is derived from the live registered object `obj`.
        unsafe {
            (*header).set_color(GcColor::Black);
        }

        // 执行 mark —— 应重置为白色（非根对象）
        gc.mark();

        // 非根对象 → 白色
        // SAFETY: `mark` does not sweep; `obj` therefore remains allocated.
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

    #[test]
    fn pending_finalizers_are_seeded_as_identity_checked_roots() {
        let mut gc = GarbageCollector::new();
        let userdata = gc.create(Userdata::new(0));
        gc.pending_finalizers.push(userdata);

        let report = gc.begin_mark_only();
        assert_eq!(report.pending_finalizers_seeded, 1);
        assert_eq!(report.pending_finalizers_rejected, 0);
        gc.propagate_marks();

        // SAFETY: begin_mark_only is non-destructive and the handle remains
        // registered in this collector.
        assert!(unsafe { &*userdata.as_ptr() }.gc_header().is_black());
    }
}

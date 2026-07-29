//! 垃圾回收器核心
//!
//! `GarbageCollector` 管理所有 GC 对象的生命周期，实现三色标记-清除算法。
//! Phase 1.3 补全了完整的标记传播、清扫回收、弱表清理和终结器框架。
//!

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ptr::NonNull;

use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::{ErasedGcRef, GcRef};
use crate::gc::header::GcObjectHeader;
use crate::gc::header::bits;
use crate::gc::object_id::ObjectId;
use crate::gc::strategy::{GcStrategy, MarkSweepGc};
use crate::gc_string::GcString;
use crate::string_pool::StringPool;
use crate::table::Table;
use crate::types::{GcColor, GcObjectType};
use crate::userdata::Userdata;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveAllocation {
    pub(crate) object_id: ObjectId,
    pub(crate) object_type: GcObjectType,
}

/// Why a copied GC handle could not be validated by one collector.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum GcRefValidationError {
    /// A scoped managed-object read was attempted without its owner collector.
    #[error("managed object access requires an active GarbageCollector")]
    CollectorUnavailable,
    /// A production string publication attempted to bypass canonical interning.
    #[error("canonical Lua string publication requires an active StringPool")]
    StringPoolUnavailable,
    /// Null is not a registered object.
    #[error("null GC reference")]
    Null,
    /// No live allocation currently occupies the candidate address.
    #[error("GC object {object_id:?} is not live in this collector")]
    NotLive {
        /// Identity carried by the rejected handle.
        object_id: ObjectId,
    },
    /// The address is live, but belongs to a different allocation identity.
    #[error(
        "GC address identity mismatch: handle requested {requested:?}, live allocation is {live:?}"
    )]
    IdentityMismatch {
        /// Identity carried by the rejected handle.
        requested: ObjectId,
        /// Identity registered at the address now.
        live: ObjectId,
    },
    /// The identity is live but its concrete allocation tag differs.
    #[error("GC object type mismatch: expected {expected:?}, found {actual:?}")]
    TypeMismatch {
        /// Concrete tag required by the typed handle.
        expected: GcObjectType,
        /// Concrete tag in the side table.
        actual: GcObjectType,
    },
}

/// One object destroyed by deterministic collector shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DestroyedGcObject {
    /// Concrete layout dispatched by the collector.
    pub object_type: GcObjectType,
    /// Whether the object carried the fixed bit at destruction time.
    pub was_fixed: bool,
}

/// Result of an explicit, non-collecting collector teardown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcDestroyAllReport {
    /// Destruction dispatch sequence, allowing shutdown order to be asserted.
    pub destruction_order: Vec<DestroyedGcObject>,
    /// Non-fixed Thread objects destroyed in the first pass.
    pub destroyed_threads: usize,
    /// Other non-fixed objects destroyed after Threads.
    pub destroyed_other_non_fixed: usize,
    /// Fixed objects destroyed in the final pass.
    pub destroyed_fixed: usize,
    /// Entries discarded from the pending-finalizer queue without invoking
    /// Lua-visible `__gc` callbacks.
    pub pending_finalizers_discarded: usize,
}

impl GcDestroyAllReport {
    /// Total objects whose concrete Rust allocation was dropped.
    pub fn destroyed_objects(&self) -> usize {
        self.destruction_order.len()
    }

    /// Count destroyed objects with one concrete collector tag.
    pub fn destroyed_count(&self, object_type: GcObjectType) -> usize {
        self.destruction_order
            .iter()
            .filter(|entry| entry.object_type == object_type)
            .count()
    }
}

/// 垃圾回收器
///
/// 管理侵入式 GC 对象链表和根集合。
/// Phase 1.3 补全了完整的三色标记-清除循环。
pub struct GarbageCollector {
    /// 所有 GC 对象的侵入式链表头
    pub(crate) all_objects: *mut GcObjectHeader,

    /// 根对象集合（受保护，不被回收）
    pub(crate) roots: Vec<ErasedGcRef>,

    /// Objects protected while a Rust-side construction graph is not yet
    /// published into collector-visible owners.
    pub(crate) temporary_roots: HashMap<u64, ErasedGcRef>,

    /// Next collector-local lexical-root identity. Zero is never issued.
    pub(crate) next_temporary_root_id: u64,

    /// Exact-id release attempts that did not match an active temporary root.
    pub(crate) rejected_temporary_root_releases: usize,

    /// Authoritative liveness/provenance table.
    ///
    /// Candidate pointers are used only as integer lookup keys. Their object
    /// memory is never read until ObjectId and concrete tag also match.
    pub(crate) live_allocations: HashMap<usize, LiveAllocation>,

    /// 灰色对象列表（待处理的标记工作队列）
    pub(crate) gray_list: Vec<*mut GcObjectHeader>,

    /// 本轮标记中发现的弱表
    pub(crate) weak_tables: Vec<GcRef<Table>>,

    /// 当前进程生命周期内是否创建/标记过弱表。
    ///
    /// 自动弱表清理会用它避免在没有弱表的程序中频繁全堆扫描。
    pub(crate) weak_table_seen: bool,

    /// 等待执行 `__gc` 终结器的 userdata（Phase 1.4+ 启用）
    pub(crate) pending_finalizers: Vec<GcRef<Userdata>>,

    /// 本轮标记中已遍历的外部 collector 对象
    pub(crate) external_marked: Vec<ErasedGcRef>,

    /// Child/reference edges rejected during the current mark traversal.
    ///
    /// A candidate is rejected before dereference when it is not linked into
    /// this collector's live intrusive list. Cross-collector edges are not
    /// traversed: the owning collector cannot keep a foreign allocation alive.
    pub(crate) rejected_mark_edges: usize,

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
            temporary_roots: HashMap::new(),
            next_temporary_root_id: 1,
            rejected_temporary_root_releases: 0,
            live_allocations: HashMap::new(),
            gray_list: Vec::new(),
            weak_tables: Vec::new(),
            weak_table_seen: false,
            pending_finalizers: Vec::new(),
            external_marked: Vec::new(),
            rejected_mark_edges: 0,
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
    /// `GcObject` is sealed to the concrete layouts understood by the
    /// collector's tag-based mark/sweep dispatcher. Layout and tag invariants
    /// are checked before the allocation is registered.
    pub fn create<T: GcObject>(&mut self, obj: T) -> GcRef<T> {
        let object_ptr = std::ptr::from_ref(&obj).cast::<GcObjectHeader>();
        let header_ptr = std::ptr::from_ref(obj.gc_header());
        assert_eq!(
            header_ptr, object_ptr,
            "GcObject header must be the first field of its concrete object"
        );
        assert_eq!(
            obj.gc_header().gc_type(),
            T::expected_gc_type(),
            "GcObject header tag does not match its concrete object type"
        );

        let object_size = obj.get_size();
        let next_object_count = self
            .object_count
            .checked_add(1)
            .expect("GC object count overflow");
        let next_total_memory = self
            .total_memory
            .checked_add(object_size)
            .expect("GC memory accounting overflow");
        let object_id = ObjectId::allocate();
        let mut boxed = Box::new(obj);
        let raw = NonNull::from(&mut *boxed);

        // SAFETY: `GcObject` is sealed and create validated header offset 0.
        let header_ptr = raw.as_ptr().cast::<GcObjectHeader>();
        let address = header_ptr as usize;
        match self.live_allocations.entry(address) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(LiveAllocation {
                    object_id,
                    object_type: T::expected_gc_type(),
                });
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                panic!("new GC allocation reused an address that is still registered");
            }
        }

        // 加入侵入式链表
        // SAFETY: header_ptr points into `boxed`, which is converted to a raw
        // allocation before this function returns and therefore does not move.
        unsafe {
            (*header_ptr).set_next(self.all_objects);
            (*header_ptr).set_color(GcColor::White);
        }
        self.all_objects = header_ptr;
        let raw = Box::into_raw(boxed);

        self.object_count = next_object_count;
        self.total_memory = next_total_memory;

        // SAFETY: the side table and intrusive list now register this exact
        // allocation identity and concrete layout.
        unsafe {
            GcRef::from_registered(
                NonNull::new(raw).expect("Box::into_raw never returns null"),
                object_id,
            )
        }
    }

    /// 创建并添加到根集
    pub fn create_root<T: GcObject>(&mut self, obj: T) -> GcRef<T> {
        let gc_ref = self.create(obj);
        self.add_root(gc_ref);
        gc_ref
    }

    // ── 根集管理 ──────────────────────────────────────────────

    /// 添加 GC 对象到根集
    pub fn add_root<T: GcObject>(&mut self, gc_ref: GcRef<T>) {
        if self.validate_ref(gc_ref).is_err() {
            return;
        }
        let erased = gc_ref.erase();
        if !self.roots.contains(&erased) {
            self.roots.push(erased);
        }
    }

    /// 从根集移除 GC 对象
    pub fn remove_root<T: GcObject>(&mut self, gc_ref: GcRef<T>) {
        let erased = gc_ref.erase();
        self.roots.retain(|root| *root != erased);
    }

    /// 检查对象是否为根
    pub fn is_root<T: GcObject>(&self, gc_ref: GcRef<T>) -> bool {
        self.roots.contains(&gc_ref.erase())
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
        assert!(
            self.temporary_roots.is_empty(),
            "cannot clear collector while publication roots are active"
        );
        // 清空所有列表
        self.roots.clear();
        self.temporary_roots.clear();
        self.rejected_temporary_root_releases = 0;
        self.gray_list.clear();
        self.weak_tables.clear();
        self.weak_table_seen = false;
        self.pending_finalizers.clear();
        self.external_marked.clear();
        self.rejected_mark_edges = 0;

        // 遍历链表，删除所有非固定对象
        let mut prev: *mut GcObjectHeader = std::ptr::null_mut();
        let mut current = self.all_objects;

        while !current.is_null() {
            assert!(
                self.contains_object(current),
                "intrusive-list object lacks provenance entry"
            );
            // SAFETY: address membership was established before reading this
            // internal list node.
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

    /// Deterministically destroy every registered allocation for Runtime
    /// shutdown.
    ///
    /// This is not a Lua collection cycle: it performs no marking, sweeping,
    /// weak-table semantics, finalizer discovery, or Lua-visible `__gc`
    /// callback. A live StringPool is required so each String can be removed
    /// before its allocation is dropped. Destruction follows the C++ shutdown
    /// dependency order: non-fixed Threads first, other non-fixed objects
    /// second, and fixed objects last.
    pub fn destroy_all(&mut self, string_pool: &mut StringPool) -> GcDestroyAllReport {
        assert!(
            self.temporary_roots.is_empty(),
            "cannot destroy collector while publication roots are active"
        );
        let mut report = GcDestroyAllReport {
            pending_finalizers_discarded: self.pending_finalizers.len(),
            ..GcDestroyAllReport::default()
        };

        self.destroy_matching_for_shutdown(string_pool, &mut report, |object_type, is_fixed| {
            object_type == GcObjectType::Thread && !is_fixed
        });
        self.destroy_matching_for_shutdown(string_pool, &mut report, |_, is_fixed| !is_fixed);
        self.destroy_matching_for_shutdown(string_pool, &mut report, |_, is_fixed| is_fixed);

        assert!(
            self.live_allocations.is_empty(),
            "shutdown unlinked objects without removing all provenance entries"
        );
        self.all_objects = std::ptr::null_mut();
        self.roots.clear();
        self.temporary_roots.clear();
        self.rejected_temporary_root_releases = 0;
        self.gray_list.clear();
        self.weak_tables.clear();
        self.weak_table_seen = false;
        self.pending_finalizers.clear();
        self.external_marked.clear();
        self.rejected_mark_edges = 0;
        self.finalizers_running = false;
        self.object_count = 0;
        self.total_memory = 0;
        string_pool.clear();

        report
    }

    fn destroy_matching_for_shutdown(
        &mut self,
        string_pool: &mut StringPool,
        report: &mut GcDestroyAllReport,
        mut should_destroy: impl FnMut(GcObjectType, bool) -> bool,
    ) {
        let mut previous: *mut GcObjectHeader = std::ptr::null_mut();
        let mut current = self.all_objects;

        while !current.is_null() {
            let live = self
                .live_allocations
                .get(&(current as usize))
                .copied()
                .expect("intrusive-list object lacks provenance entry");
            // SAFETY: address membership was established before reading this
            // internal list node.
            let (next, is_fixed) = unsafe { ((*current).next(), (*current).is_fixed()) };
            let object_type = live.object_type;

            if should_destroy(object_type, is_fixed) {
                if previous.is_null() {
                    self.all_objects = next;
                } else {
                    // SAFETY: previous is the retained predecessor of current.
                    unsafe {
                        (*previous).set_next(next);
                    }
                }

                report.destruction_order.push(DestroyedGcObject {
                    object_type,
                    was_fixed: is_fixed,
                });
                if is_fixed {
                    report.destroyed_fixed += 1;
                } else if object_type == GcObjectType::Thread {
                    report.destroyed_threads += 1;
                } else {
                    report.destroyed_other_non_fixed += 1;
                }
                self.destroy_object(current, string_pool);
            } else {
                previous = current;
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
        let mut roots = self.roots.clone();
        roots.extend(self.temporary_roots.values().copied());
        for root in roots {
            match self.validate_erased(root) {
                Ok(pointer) => self.mark_live_object(pointer.as_ptr()),
                Err(_) => {
                    self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                }
            }
        }
    }

    /// Mark an already validated collector-owned object.
    pub(crate) fn mark_live_object(&mut self, obj: *mut GcObjectHeader) {
        if !self.contains_object(obj) {
            self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
            return;
        }
        // SAFETY: callers obtain `obj` from the authoritative live-allocation
        // table or the collector-owned intrusive list.
        unsafe {
            if !(*obj).is_white() {
                return;
            }
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

    /// Number of explicit and lexical publication roots currently protected.
    pub fn total_protected_root_count(&self) -> usize {
        self.roots.len() + self.temporary_roots.len()
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

    /// Return whether an address is currently registered in this collector.
    ///
    /// The candidate is converted to an integer lookup key and is never
    /// dereferenced. Raw-address validation is reserved for collector-internal
    /// work queues; external handles must also match identity and type.
    pub(crate) fn contains_object(&self, candidate: *mut GcObjectHeader) -> bool {
        !candidate.is_null() && self.live_allocations.contains_key(&(candidate as usize))
    }

    /// Validate a typed handle against this collector's authoritative side
    /// table without reading candidate object memory.
    ///
    pub fn validate_ref<T: GcObject>(
        &self,
        value: GcRef<T>,
    ) -> Result<NonNull<T>, GcRefValidationError> {
        let pointer = value.as_nonnull().ok_or(GcRefValidationError::Null)?;
        if value.object_id().is_null() {
            return Err(GcRefValidationError::Null);
        }
        let live = self
            .live_allocations
            .get(&(pointer.as_ptr() as usize))
            .ok_or(GcRefValidationError::NotLive {
                object_id: value.object_id(),
            })?;
        if live.object_id != value.object_id() {
            return Err(GcRefValidationError::IdentityMismatch {
                requested: value.object_id(),
                live: live.object_id,
            });
        }
        let expected = T::expected_gc_type();
        if live.object_type != expected {
            return Err(GcRefValidationError::TypeMismatch {
                expected,
                actual: live.object_type,
            });
        }
        Ok(pointer)
    }

    /// Borrow a validated object for exactly the duration of `read`.
    pub fn with_ref<T: GcObject, R>(
        &self,
        value: GcRef<T>,
        read: impl for<'a> FnOnce(&'a T) -> R,
    ) -> Result<R, GcRefValidationError> {
        let pointer = self.validate_ref(value)?;
        // SAFETY: the side table matched address, identity, and concrete tag.
        // `&self` prevents this collector from destroying the allocation while
        // the callback is running.
        Ok(read(unsafe { pointer.as_ref() }))
    }

    /// Mutably borrow a validated object for exactly the duration of `write`.
    pub fn with_mut<T: GcObject, R>(
        &mut self,
        value: GcRef<T>,
        write: impl for<'a> FnOnce(&'a mut T) -> R,
    ) -> Result<R, GcRefValidationError> {
        let mut pointer = self.validate_ref(value)?;
        // SAFETY: the side table matched address, identity, and concrete tag.
        // `&mut self` prevents collection and other collector-mediated
        // mutation while the callback is running.
        Ok(write(unsafe { pointer.as_mut() }))
    }

    /// Read a live Lua string's bytes for exactly the duration of `read`.
    ///
    /// Validation happens before object memory is touched, so foreign, stale,
    /// wrong-type, and address-reused handles fail closed.
    pub fn with_string_bytes<R>(
        &self,
        value: GcRef<GcString>,
        read: impl for<'a> FnOnce(&'a [u8]) -> R,
    ) -> Result<R, GcRefValidationError> {
        self.with_ref(value, |string| read(string.as_bytes()))
    }

    /// Compare two collector-owned strings by exact Lua byte content.
    ///
    /// Normal `Value::String` equality is canonical allocation identity. This
    /// scoped operation is reserved for boundaries that intentionally receive
    /// non-canonical handles.
    pub fn string_refs_equal(
        &self,
        left: GcRef<GcString>,
        right: GcRef<GcString>,
    ) -> Result<bool, GcRefValidationError> {
        let left = self.with_string_bytes(left, <[u8]>::to_vec)?;
        self.with_string_bytes(right, |right| left.as_slice() == right)
    }

    /// Order two collector-owned strings by exact unsigned byte lexicography.
    pub fn compare_string_refs(
        &self,
        left: GcRef<GcString>,
        right: GcRef<GcString>,
    ) -> Result<Ordering, GcRefValidationError> {
        let left = self.with_string_bytes(left, <[u8]>::to_vec)?;
        self.with_string_bytes(right, |right| left.as_slice().cmp(right))
    }

    pub(crate) fn validate_erased(
        &self,
        value: ErasedGcRef,
    ) -> Result<NonNull<GcObjectHeader>, GcRefValidationError> {
        let pointer = NonNull::new(value.ptr()).ok_or(GcRefValidationError::Null)?;
        if value.object_id().is_null() {
            return Err(GcRefValidationError::Null);
        }
        let live = self
            .live_allocations
            .get(&(pointer.as_ptr() as usize))
            .ok_or(GcRefValidationError::NotLive {
                object_id: value.object_id(),
            })?;
        if live.object_id != value.object_id() {
            return Err(GcRefValidationError::IdentityMismatch {
                requested: value.object_id(),
                live: live.object_id,
            });
        }
        if live.object_type != value.object_type() {
            return Err(GcRefValidationError::TypeMismatch {
                expected: value.object_type(),
                actual: live.object_type,
            });
        }
        Ok(pointer)
    }

    /// Return whether a typed handle names the exact live allocation in this
    /// collector.
    pub fn contains_registered<T: GcObject>(&self, value: GcRef<T>) -> bool {
        self.validate_ref(value).is_ok()
    }

    /// Reconstitute a typed handle for a raw pointer obtained from a
    /// collector-owned intrusive queue.
    pub(crate) fn registered_ref_from_ptr<T: GcObject>(&self, pointer: *mut T) -> Option<GcRef<T>> {
        let pointer = NonNull::new(pointer)?;
        let live = self.live_allocations.get(&(pointer.as_ptr() as usize))?;
        if live.object_type != T::expected_gc_type() {
            return None;
        }
        // SAFETY: the authoritative table proves this address, identity, and
        // concrete type are currently registered.
        Some(unsafe { GcRef::from_registered(pointer, live.object_id) })
    }

    /// Count objects reached by the current mark-only traversal.
    pub fn marked_object_count(&self) -> usize {
        let mut count = 0;
        self.for_each_object(|object| {
            // SAFETY: `for_each_object` yields only linked collector objects.
            if unsafe { !(*object).is_white() } {
                count += 1;
            }
        });
        count
    }

    /// Return the number of objects still waiting in the gray queue.
    pub fn pending_mark_count(&self) -> usize {
        self.gray_list.len()
    }

    /// Return child/reference edges rejected in the current mark traversal.
    pub fn rejected_mark_edge_count(&self) -> usize {
        self.rejected_mark_edges
    }

    /// Number of entries retained by transient collector work/finalizer
    /// queues, excluding the explicit root vector.
    pub fn transient_queue_entry_count(&self) -> usize {
        self.gray_list.len()
            + self.weak_tables.len()
            + self.pending_finalizers.len()
            + self.external_marked.len()
    }

    /// Number of userdata entries waiting in the finalizer queue.
    pub fn pending_finalizer_count(&self) -> usize {
        self.pending_finalizers.len()
    }

    /// Register a table for weak-entry cleanup without running a full GC cycle.
    pub fn register_weak_table(&mut self, table: GcRef<Table>, weak_keys: bool, weak_values: bool) {
        if !weak_keys && !weak_values {
            return;
        }
        let Ok(pointer) = self.validate_ref(table) else {
            return;
        };
        self.weak_table_seen = true;
        let ptr = pointer.as_ptr().cast::<GcObjectHeader>();

        // SAFETY: validation matched this collector, allocation identity, and
        // the Table tag before the header is read.
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

        if !self.weak_tables.contains(&table) {
            self.weak_tables.push(table);
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
    pub fn is_object_dead<T: GcObject>(&self, value: GcRef<T>) -> bool {
        let Ok(pointer) = self.validate_ref(value) else {
            return true;
        };
        let obj = pointer.as_ptr().cast::<GcObjectHeader>();
        // SAFETY: validation matched address, identity, and concrete tag.
        unsafe {
            if (*obj).is_fixed() {
                return false;
            }
            (*obj).is_white()
        }
    }

    /// 检查包含 GC 引用的 Value 中的对象是否已死
    ///
    pub fn is_value_dead(&self, value: &crate::value::Value) -> bool {
        match value {
            crate::value::Value::String(s) => !self.contains_registered(*s),
            crate::value::Value::Table(t) => self.is_object_dead(*t),
            crate::value::Value::Function(f) => self.is_object_dead(*f),
            crate::value::Value::Userdata(u) => self.is_object_dead(*u),
            crate::value::Value::Thread(t) => self.is_object_dead(*t),
            // Nil, Boolean, Number, LightUserdata 不是 GC 对象
            _ => false,
        }
    }

    /// 检查弱值槽位是否应被清理
    ///
    /// 字符串永远不会被清理；userdata 在 pending_finalizers 中时视为已死。
    ///
    pub fn is_weak_value_dead(&self, value: &crate::value::Value) -> bool {
        match value {
            crate::value::Value::String(value) => !self.contains_registered(*value),
            crate::value::Value::Userdata(u) => {
                let Ok(pointer) = self.validate_ref(*u) else {
                    return true;
                };
                let ptr = pointer.as_ptr().cast::<GcObjectHeader>();
                if self.pending_finalizers.contains(u) {
                    return true;
                }
                // Once a userdata finalizer has run, weak-value slots should be
                // cleared on the next GC cycle even though this compatibility
                // collector does not immediately sweep the userdata object.
                // SAFETY: validation matched allocation identity and Userdata
                // tag before the header is read.
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
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::function::Function;
    use crate::gc_string::GcString;
    use crate::proto::Proto;
    use crate::string_pool::StringPool;
    use crate::table::Table;
    use crate::thread::Thread;
    use crate::upvalue::Upvalue;
    use crate::userdata::Userdata;
    use crate::value::Value;

    #[test]
    fn test_create_object() {
        let mut gc = GarbageCollector::new();
        let gc_ref: GcRef<Table> = gc.create(Table::new());

        assert!(!gc_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_add_remove_root() {
        let mut gc = GarbageCollector::new();
        let gc_ref = gc.create(Table::new());

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
        let gc_ref = gc.create_root(Table::new());

        assert!(gc.is_root(gc_ref));
        assert_eq!(gc.object_count(), 1);
        assert_eq!(gc.root_count(), 1);
    }

    #[test]
    fn validation_rejects_foreign_stale_and_type_confused_handles_before_dereference() {
        let mut owner = GarbageCollector::new();
        let foreign = GarbageCollector::new();
        let mut pool = StringPool::new();

        let table = owner.create(Table::new());
        assert!(matches!(
            foreign.validate_ref(table),
            Err(GcRefValidationError::NotLive { .. })
        ));

        let wrong_pointer = NonNull::new(table.as_ptr().cast_mut().cast::<GcString>())
            .expect("a registered handle is non-null");
        // SAFETY: this synthetic handle is used only to exercise side-table
        // validation and is never dereferenced.
        let wrong_type = unsafe { GcRef::from_registered(wrong_pointer, table.object_id()) };
        assert!(matches!(
            owner.validate_ref(wrong_type),
            Err(GcRefValidationError::TypeMismatch {
                expected: GcObjectType::String,
                actual: GcObjectType::Table,
            })
        ));

        assert_eq!(owner.sweep(&mut pool), 1);
        assert!(matches!(
            owner.validate_ref(table),
            Err(GcRefValidationError::NotLive { .. })
        ));
    }

    #[test]
    fn address_reuse_injection_rejects_old_object_id_at_same_pointer() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let original = gc.create(Table::new());
        let address = original.as_ptr() as usize;
        let replacement_id = ObjectId::allocate();

        // Test-only allocator-reuse injection: preserve the live allocation
        // and address while replacing its authoritative identity.
        gc.live_allocations
            .get_mut(&address)
            .expect("allocation is registered")
            .object_id = replacement_id;

        assert!(matches!(
            gc.validate_ref(original),
            Err(GcRefValidationError::IdentityMismatch {
                requested,
                live,
            }) if requested == original.object_id() && live == replacement_id
        ));

        let pointer =
            NonNull::new(original.as_ptr().cast_mut()).expect("registered pointer is non-null");
        // SAFETY: the test-injected side-table entry now assigns
        // `replacement_id` to this live Table allocation.
        let replacement = unsafe { GcRef::from_registered(pointer, replacement_id) };
        assert_eq!(gc.validate_ref(replacement).unwrap(), pointer);

        assert_eq!(gc.sweep(&mut pool), 1);
        assert!(gc.live_allocations.is_empty());
    }

    #[test]
    fn scoped_string_comparison_rejects_foreign_stale_and_reused_identity() {
        let mut owner = GarbageCollector::new();
        let mut owner_pool = StringPool::new();
        let left = owner.create(GcString::from_bytes(&[b'a', 0, 0x80, 0xff]));
        let duplicate = owner.create(GcString::from_bytes(&[b'a', 0, 0x80, 0xff]));
        let different = owner.create(GcString::from_bytes(&[b'a', 0, 0x80, 0xfe]));

        assert_eq!(owner.string_refs_equal(left, duplicate), Ok(true));
        assert_eq!(
            owner.compare_string_refs(left, different),
            Ok(std::cmp::Ordering::Greater)
        );

        let mut foreign = GarbageCollector::new();
        let foreign_string = foreign.create(GcString::from_bytes(&[b'a', 0, 0x80, 0xff]));
        assert_ne!(Value::String(left), Value::String(foreign_string));
        assert!(matches!(
            owner.string_refs_equal(left, foreign_string),
            Err(GcRefValidationError::NotLive { .. })
        ));

        let reused_address = left.as_ptr() as usize;
        let value = Value::String(left);
        let mut before = DefaultHasher::new();
        value.hash(&mut before);
        let hash_before_reuse = before.finish();
        let replacement_id = ObjectId::allocate();
        owner
            .live_allocations
            .get_mut(&reused_address)
            .expect("string allocation is registered")
            .object_id = replacement_id;
        assert!(matches!(
            owner.with_string_bytes(left, <[u8]>::to_vec),
            Err(GcRefValidationError::IdentityMismatch {
                requested,
                live,
            }) if requested == left.object_id() && live == replacement_id
        ));
        let replacement_pointer =
            NonNull::new(left.as_ptr().cast_mut()).expect("string address remains non-null");
        // SAFETY: the test side table currently assigns `replacement_id` to
        // this address, so this handle models an actual address reuse.
        let replacement =
            unsafe { GcRef::<GcString>::from_registered(replacement_pointer, replacement_id) };
        assert_ne!(value, Value::String(replacement));
        let mut after = DefaultHasher::new();
        value.hash(&mut after);
        assert_eq!(after.finish(), hash_before_reuse);

        // Restore the identity so normal sweeping can destroy the allocation.
        owner
            .live_allocations
            .get_mut(&reused_address)
            .expect("string allocation remains registered")
            .object_id = left.object_id();
        assert_eq!(owner.sweep(&mut owner_pool), 3);
        assert!(matches!(
            owner.with_string_bytes(duplicate, <[u8]>::to_vec),
            Err(GcRefValidationError::NotLive { .. })
        ));
        let mut stale_hash = DefaultHasher::new();
        value.hash(&mut stale_hash);
        assert_eq!(stale_hash.finish(), hash_before_reuse);

        let mut foreign_pool = StringPool::new();
        assert_eq!(foreign.sweep(&mut foreign_pool), 1);
    }

    #[test]
    fn persistent_queues_do_not_reidentify_reused_addresses() {
        let mut gc = GarbageCollector::new();
        let table = gc.create(Table::new());
        let userdata = gc.create(Userdata::new(0));
        let table_address = table.as_ptr() as usize;
        let userdata_address = userdata.as_ptr() as usize;
        gc.weak_tables.push(table);
        gc.pending_finalizers.push(userdata);

        // Keep the replacement userdata alive so only an erroneous
        // address-only pending-finalizer match could report it dead.
        // SAFETY: userdata is live and registered in this collector.
        unsafe {
            (*(userdata.as_ptr() as *mut GcObjectHeader)).set_color(GcColor::Black);
        }

        let table_replacement_id = ObjectId::allocate();
        let userdata_replacement_id = ObjectId::allocate();
        gc.live_allocations
            .get_mut(&table_address)
            .unwrap()
            .object_id = table_replacement_id;
        gc.live_allocations
            .get_mut(&userdata_address)
            .unwrap()
            .object_id = userdata_replacement_id;

        // SAFETY: these handles reflect the test-injected authoritative
        // identities and are used only while the allocations remain live.
        let replacement_table = unsafe {
            GcRef::from_registered(
                NonNull::new(table.as_ptr().cast_mut()).unwrap(),
                table_replacement_id,
            )
        };
        // SAFETY: same as above, for the live Userdata allocation.
        let replacement_userdata = unsafe {
            GcRef::from_registered(
                NonNull::new(userdata.as_ptr().cast_mut()).unwrap(),
                userdata_replacement_id,
            )
        };

        assert!(!gc.weak_tables.contains(&replacement_table));
        assert!(!gc.pending_finalizers.contains(&replacement_userdata));
        assert!(!gc.is_weak_value_dead(&Value::Userdata(replacement_userdata)));

        let rejected_before = gc.rejected_mark_edge_count();
        gc.clear_registered_weak_tables();
        assert!(gc.rejected_mark_edge_count() > rejected_before);
    }

    #[test]
    fn roots_reject_foreign_handles_without_retaining_raw_addresses() {
        let mut owner = GarbageCollector::new();
        let mut foreign = GarbageCollector::new();
        let foreign_table = foreign.create(Table::new());

        owner.add_root(foreign_table);

        assert_eq!(owner.root_count(), 0);
        assert!(!owner.is_root(foreign_table));
    }

    #[test]
    fn checked_borrow_helpers_validate_before_running_callbacks() {
        let mut gc = GarbageCollector::new();
        let table = gc.create(Table::new());

        gc.with_mut(table, |value| {
            value.set_array(1, &Value::Number(42.0));
        })
        .unwrap();
        assert_eq!(
            gc.with_ref(table, |value| value.get_array(1)).unwrap(),
            Value::Number(42.0)
        );

        let foreign = GarbageCollector::new();
        let mut callback_ran = false;
        assert!(
            foreign
                .with_ref(table, |_| {
                    callback_ran = true;
                })
                .is_err()
        );
        assert!(!callback_ran);
    }

    #[test]
    fn test_reset_marks() {
        let mut gc = GarbageCollector::new();
        let gc_ref = gc.create(Table::new());

        // 设置颜色为黑色
        // SAFETY: `gc_ref` was allocated by `gc` and no collection occurs
        // while its header is accessed; `GcObject` requires header offset 0.
        unsafe {
            let header_ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
            (*header_ptr).set_color(GcColor::Black);
            assert!((*header_ptr).is_black());
        }

        gc.reset_marks();
        // SAFETY: `gc_ref` remains owned by `gc`; `reset_marks` changes only
        // mark bits and does not free or relocate the object.
        unsafe {
            let header_ptr = gc_ref.as_ptr() as *mut GcObjectHeader;
            assert!((*header_ptr).is_white());
        }
    }

    #[test]
    fn test_object_count_tracking() {
        let mut gc = GarbageCollector::new();
        assert_eq!(gc.object_count(), 0);

        gc.create(Table::new());
        gc.create(Table::new());
        gc.create(Table::new());

        assert_eq!(gc.object_count(), 3);
    }

    #[test]
    fn test_create_accepts_every_builtin_dispatch_layout() {
        unsafe extern "C" fn noop(_state: *mut std::ffi::c_void) -> i32 {
            0
        }

        let mut gc = GarbageCollector::new();
        gc.create(GcString::from_bytes(b"string"));
        gc.create(Table::new());
        gc.create(Function::new_c(noop));
        gc.create(Userdata::new(0));
        gc.create(Thread::new());
        gc.create(Proto::new());
        gc.create(Upvalue::new_closed(Value::Nil));

        assert_eq!(gc.object_count(), 7);
    }

    #[test]
    fn destroy_all_drops_every_layout_thread_first_and_fixed_last() {
        unsafe extern "C" fn noop(_state: *mut std::ffi::c_void) -> i32 {
            0
        }

        unsafe fn count_probe(payload: *mut u8) {
            let mut encoded = [0_u8; std::mem::size_of::<usize>()];
            // SAFETY: the test stores exactly one native-endian usize in the
            // userdata byte payload before registering this callback.
            unsafe {
                std::ptr::copy_nonoverlapping(payload, encoded.as_mut_ptr(), encoded.len());
            }
            let counter = usize::from_ne_bytes(encoded) as *const AtomicUsize;
            // SAFETY: the pointed-to test counter outlives collector teardown.
            unsafe {
                (*counter).fetch_add(1, Ordering::SeqCst);
            }
        }

        fn probe(counter: &AtomicUsize) -> Userdata {
            let mut userdata = Userdata::new(std::mem::size_of::<usize>());
            userdata
                .data_mut()
                .copy_from_slice(&(std::ptr::from_ref(counter) as usize).to_ne_bytes());
            // SAFETY: count_probe reads only the encoded AtomicUsize pointer;
            // the counter outlives the synchronous destroy_all call.
            unsafe {
                userdata.set_destructor(count_probe);
            }
            userdata
        }

        let ordinary_probe_drops = AtomicUsize::new(0);
        let fixed_probe_drops = AtomicUsize::new(0);
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let string = pool.intern_bytes(&mut gc, b"shutdown");
        let table = gc.create(Table::new());
        gc.create(Function::new_c(noop));
        let ordinary_probe = gc.create(probe(&ordinary_probe_drops));
        gc.create(Thread::new());
        gc.create(Proto::new());
        gc.create(Upvalue::new_closed(Value::Nil));
        let fixed_probe = gc.create(probe(&fixed_probe_drops));

        // SAFETY: all three objects are live headers registered in `gc`.
        unsafe {
            (*(fixed_probe.as_ptr() as *const GcObjectHeader)).mark_fixed();
            (*(string.as_ptr() as *const GcObjectHeader)).mark_fixed();
        }
        gc.mark_registered(table);
        gc.add_root(string);
        gc.register_weak_table(table, true, false);
        gc.pending_finalizers.push(ordinary_probe);
        gc.external_marked.push(fixed_probe.erase());

        let report = gc.destroy_all(&mut pool);

        assert_eq!(report.destroyed_objects(), 8);
        for object_type in [
            GcObjectType::String,
            GcObjectType::Table,
            GcObjectType::Function,
            GcObjectType::Thread,
            GcObjectType::Proto,
            GcObjectType::Upval,
        ] {
            assert_eq!(report.destroyed_count(object_type), 1);
        }
        assert_eq!(report.destroyed_count(GcObjectType::Userdata), 2);
        assert_eq!(report.destroyed_threads, 1);
        assert_eq!(report.destroyed_other_non_fixed, 5);
        assert_eq!(report.destroyed_fixed, 2);
        assert_eq!(report.pending_finalizers_discarded, 1);
        assert_eq!(
            report.destruction_order.first(),
            Some(&DestroyedGcObject {
                object_type: GcObjectType::Thread,
                was_fixed: false,
            })
        );
        assert_eq!(
            report.destruction_order.last(),
            Some(&DestroyedGcObject {
                object_type: GcObjectType::String,
                was_fixed: true,
            })
        );
        assert_eq!(ordinary_probe_drops.load(Ordering::SeqCst), 1);
        assert_eq!(fixed_probe_drops.load(Ordering::SeqCst), 1);
        assert!(gc.all_objects.is_null());
        assert_eq!(gc.object_count(), 0);
        assert_eq!(gc.total_memory(), 0);
        assert_eq!(gc.root_count(), 0);
        assert_eq!(gc.transient_queue_entry_count(), 0);
        assert!(pool.is_empty());
    }
}

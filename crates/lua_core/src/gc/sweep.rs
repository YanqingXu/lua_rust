//! GC 清扫阶段实现
//!
//! 回收所有未标记（白色）的对象。遍历侵入式链表，
//! 移除白色对象并释放其内存，同时维护统计信息。
//!

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::gc_string::GcString;
use crate::string_pool::StringPool;
use crate::types::GcObjectType;

impl GarbageCollector {
    /// 执行清扫阶段
    ///
    /// 遍历 GC 对象链表，回收所有白色（未标记）且非固定的对象。
    /// 存活对象被重置为白色以准备下一轮 GC。
    ///
    /// 字符串对象被回收时会同步从 StringPool 中移除。
    ///
    pub fn sweep(&mut self, string_pool: &mut StringPool) -> usize {
        string_pool.bind_or_assert_owner(self.heap_id());
        let mut collected = 0;
        let mut prev: *mut GcObjectHeader = std::ptr::null_mut();
        let mut current = self.all_objects;

        while !current.is_null() {
            assert!(
                self.contains_object(current),
                "intrusive-list object lacks provenance entry"
            );
            // SAFETY: address membership was established before reading this
            // internal list node.
            let next = unsafe { (*current).next() };

            let should_sweep = {
                // SAFETY: current is valid
                let obj = unsafe { &*current };
                let is_fixed = obj.is_fixed();
                let is_white = obj.is_white();
                is_white && !is_fixed
            };

            if should_sweep {
                // 从链表中移除
                if prev.is_null() {
                    self.all_objects = next;
                } else {
                    // SAFETY: prev is a valid node
                    unsafe {
                        (*prev).set_next(next);
                    }
                }

                // 销毁对象
                self.destroy_object(current, string_pool);
                collected += 1;
                // prev 不变（当前对象已删除）
            } else {
                // 保留对象，重置为白色（为下次 GC 准备）
                // SAFETY: current is valid
                unsafe {
                    (*current).set_color(crate::types::GcColor::White);
                }
                prev = current;
            }

            current = next;
        }

        collected
    }

    /// 销毁单个 GC 对象并释放内存
    ///
    /// 从所有内部列表中移除该对象，更新统计信息，
    /// 回收内存。如果是字符串对象，同步从 StringPool 中移除。
    ///
    pub(crate) fn destroy_object(
        &mut self,
        obj: *mut GcObjectHeader,
        string_pool: &mut StringPool,
    ) {
        self.destroy_object_inner(obj, Some(string_pool));
    }

    /// Destroy one allocation when no StringPool owner is available.
    ///
    /// This is reserved for the standalone collector Drop safety net. Any
    /// external pool entry becomes a stale identity handle, which checked
    /// collector APIs reject without dereferencing.
    pub(crate) fn destroy_object_without_pool(&mut self, obj: *mut GcObjectHeader) {
        self.destroy_object_inner(obj, None);
    }

    fn destroy_object_inner(
        &mut self,
        obj: *mut GcObjectHeader,
        string_pool: Option<&mut StringPool>,
    ) {
        if obj.is_null() {
            return;
        }

        // Remove authoritative provenance before any operation that could
        // panic. From this point on every copied handle fails closed, even if
        // unwinding leaks the detached allocation.
        let live = self
            .live_allocations
            .remove(&(obj as usize))
            .expect("destroyed GC object was not registered");

        // 从内部列表中移除
        self.roots.retain(|root| root.ptr() != obj);
        self.temporary_roots
            .retain(|_, reference| reference.ptr() != obj);
        self.gray_list.retain(|&r| r != obj);
        self.weak_tables
            .retain(|reference| reference.as_ptr().cast::<GcObjectHeader>().cast_mut() != obj);
        self.pending_finalizers
            .retain(|reference| reference.as_ptr().cast::<GcObjectHeader>().cast_mut() != obj);
        self.external_marked
            .retain(|reference| reference.ptr() != obj);

        // SAFETY: `obj` came from the intrusive list and its concrete layout
        // is recorded in the authoritative side table entry just removed.
        let obj_size = unsafe { self.object_size_of(obj, live.object_type) };

        // 如果是字符串，从 StringPool 中移除
        if live.object_type == GcObjectType::String
            && let Some(string_pool) = string_pool
        {
            let pointer = std::ptr::NonNull::new(obj.cast::<GcString>())
                .expect("intrusive-list nodes are non-null");
            // SAFETY: the removed entry proves this exact allocation identity
            // and String layout; memory remains allocated until dispatch below.
            let gc_ref: GcRef<GcString> =
                unsafe { GcRef::from_registered(pointer, live.object_id) };
            string_pool.remove(gc_ref);
        }

        // 更新统计信息
        self.total_memory = self.total_memory.saturating_sub(obj_size);
        self.subtract_gc_debt(obj_size);
        if self.object_count > 0 {
            self.object_count -= 1;
        }

        // 重置 next 指针（避免悬空引用）
        // SAFETY: obj is being destroyed
        unsafe {
            (*obj).set_next(std::ptr::null_mut());
        }

        // 释放内存：通过裸指针重建 Box 并 drop
        // SAFETY: obj 通过 Box::into_raw 分配，现在回收所有权
        unsafe {
            Self::free_gc_object(obj, live.object_type);
        }
    }

    /// 获取 GC 对象的大小（用于统计更新）
    ///
    /// # Safety
    /// `obj` 必须指向有效的 GC 对象。
    unsafe fn object_size_of(&self, obj: *mut GcObjectHeader, object_type: GcObjectType) -> usize {
        // SAFETY: caller guarantees obj is valid
        unsafe {
            match object_type {
                GcObjectType::String => {
                    use crate::gc_string::GcString;
                    let ptr = obj as *const GcString;
                    (*ptr).get_size()
                }
                GcObjectType::Table => {
                    use crate::table::Table;
                    let ptr = obj as *const Table;
                    (*ptr).get_size()
                }
                GcObjectType::Function => {
                    use crate::function::Function;
                    let ptr = obj as *const Function;
                    (*ptr).get_size()
                }
                GcObjectType::Proto => {
                    use crate::proto::Proto;
                    let ptr = obj as *const Proto;
                    (*ptr).get_size()
                }
                GcObjectType::Upval => {
                    use crate::upvalue::Upvalue;
                    let ptr = obj as *const Upvalue;
                    (*ptr).get_size()
                }
                GcObjectType::Userdata => {
                    use crate::userdata::Userdata;
                    let ptr = obj as *const Userdata;
                    (*ptr).get_size()
                }
                GcObjectType::Thread => {
                    use crate::thread::Thread;
                    let ptr = obj as *const Thread;
                    (*ptr).get_size()
                }
            }
        }
    }

    /// 释放 GC 对象内存
    ///
    /// 根据对象类型，将裸指针转回对应的 Box 类型并 drop。
    ///
    /// This concrete dispatch is sound only because `GcObject` is sealed and
    /// `GarbageCollector::create` validates each allocation's leading header
    /// and tag before registration.
    ///
    /// TODO(M1 GC metadata): move this drop function (and trace/size) into
    /// type-erased per-allocation metadata before supporting extensible object
    /// types or reclaiming objects from collector `Drop`.
    ///
    /// # Safety
    /// `obj` 必须是通过 `Box::into_raw` 分配的，且尚未被释放。
    unsafe fn free_gc_object(obj: *mut GcObjectHeader, gc_type: GcObjectType) {
        // SAFETY: caller guarantees obj was allocated via Box::into_raw
        unsafe {
            match gc_type {
                GcObjectType::String => {
                    use crate::gc_string::GcString;
                    let _ = Box::from_raw(obj as *mut GcString);
                }
                GcObjectType::Table => {
                    use crate::table::Table;
                    let _ = Box::from_raw(obj as *mut Table);
                }
                GcObjectType::Function => {
                    use crate::function::Function;
                    let _ = Box::from_raw(obj as *mut Function);
                }
                GcObjectType::Proto => {
                    use crate::proto::Proto;
                    let _ = Box::from_raw(obj as *mut Proto);
                }
                GcObjectType::Upval => {
                    use crate::upvalue::Upvalue;
                    let _ = Box::from_raw(obj as *mut Upvalue);
                }
                GcObjectType::Userdata => {
                    use crate::userdata::Userdata;
                    let _ = Box::from_raw(obj as *mut Userdata);
                }
                GcObjectType::Thread => {
                    use crate::thread::Thread;
                    let _ = Box::from_raw(obj as *mut Thread);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::gc::collector::GarbageCollector;
    use crate::gc::header::GcObjectHeader;
    use crate::gc_string::GcString;
    use crate::string_pool::StringPool;
    use crate::table::Table;
    use crate::types::GcColor;
    use crate::userdata::Userdata;

    #[test]
    fn test_sweep_removes_white_objects() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let obj1 = gc.create(GcString::from_bytes(b"keep"));
        let _obj2 = gc.create(GcString::from_bytes(b"sweep"));

        // obj1 标记为黑色（存活），obj2 保持白色（应被回收）
        // SAFETY: `obj1` is a live object registered with `gc`, and the
        // `GcObject` layout places its header at offset 0.
        unsafe {
            let h1 = obj1.as_ptr() as *mut GcObjectHeader;
            (*h1).set_color(GcColor::Black);
        }

        let before = gc.object_count();
        let collected = gc.sweep(&mut pool);

        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), before - 1);
    }

    #[test]
    fn test_sweep_preserves_fixed_objects() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let fixed_obj = gc.create(Table::new());
        // 标记为固定
        // SAFETY: `fixed_obj` is live and registered with `gc`; its header is
        // the first field by the `GcObject` contract.
        unsafe {
            let header = fixed_obj.as_ptr() as *mut GcObjectHeader;
            (*header).mark_fixed();
            // 保持白色
            (*header).set_color(GcColor::White);
        }

        let before = gc.object_count();
        let collected = gc.sweep(&mut pool);

        // 固定对象不应被回收
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), before);
    }

    #[test]
    fn test_sweep_resets_survivors_to_white() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let survivor = gc.create(Table::new());
        // 标记为黑色
        // SAFETY: `survivor` is a live registered object and its header is at
        // offset 0.
        unsafe {
            let header = survivor.as_ptr() as *mut GcObjectHeader;
            (*header).set_color(GcColor::Black);
        }

        gc.sweep(&mut pool);

        // sweep 后存活对象应重置为白色
        // SAFETY: the black object survived `sweep`, which does not relocate
        // survivors, so `survivor` still points to a live table.
        unsafe {
            let header = survivor.as_ptr() as *mut GcObjectHeader;
            assert!((*header).is_white());
        }
    }

    #[test]
    fn test_sweep_removes_string_from_pool() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let _s = pool.intern_bytes(&mut gc, b"temporary_string");
        assert!(pool.find_bytes(b"temporary_string").is_some());

        // sweep（字符串为白色 → 应被回收）
        let _collected = gc.sweep(&mut pool);

        // 字符串应从池中移除
        assert!(pool.find_bytes(b"temporary_string").is_none());
    }

    #[test]
    fn test_sweep_empty_list() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 0);
    }

    #[test]
    fn panicking_drop_unregisters_provenance_before_unwind() {
        unsafe fn panic_destructor(_: *mut u8) {
            panic!("injected userdata destructor panic");
        }

        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let mut userdata = Userdata::new(0);
        // SAFETY: the injected callback does not inspect or retain its
        // payload; it exists only to exercise destruction unwinding.
        unsafe {
            userdata.set_destructor(panic_destructor);
        }
        let stale = gc.create(userdata);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gc.sweep(&mut pool);
        }));

        assert!(result.is_err());
        assert!(!gc.contains_registered(stale));
        assert_eq!(gc.object_count(), 0);
        assert_eq!(gc.total_memory(), 0);
    }
}

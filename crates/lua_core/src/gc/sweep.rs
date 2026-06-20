//! GC 清扫阶段实现
//!
//! 回收所有未标记（白色）的对象。遍历侵入式链表，
//! 移除白色对象并释放其内存，同时维护统计信息。
//!
//! C++ 参考: `lua_cpp/src/gc/gc_sweep.cpp`

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
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
    /// C++ 对应: `GarbageCollector::sweep(StringPool& stringPool)`
    pub fn sweep(&mut self, string_pool: &mut StringPool) -> usize {
        let mut collected = 0;
        let mut prev: *mut GcObjectHeader = std::ptr::null_mut();
        let mut current = self.all_objects;

        while !current.is_null() {
            // SAFETY: current is a node in the intrusive linked list
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
    /// C++ 对应: `GarbageCollector::destroyObject(GCObject* obj, StringPool& stringPool)`
    pub(crate) fn destroy_object(
        &mut self,
        obj: *mut GcObjectHeader,
        string_pool: &mut StringPool,
    ) {
        if obj.is_null() {
            return;
        }

        // 从内部列表中移除
        self.roots.retain(|&r| r != obj);
        self.gray_list.retain(|&r| r != obj);
        self.external_marked.retain(|&r| r != obj);

        // SAFETY: obj is a valid GC object pointer
        let (gc_type, obj_size) = unsafe { ((*obj).gc_type(), self.object_size_of(obj)) };

        // 如果是字符串，从 StringPool 中移除
        if gc_type == GcObjectType::String {
            use crate::gc::gc_ref::GcRef;
            use crate::gc_string::GcString;
            // SAFETY: obj is a valid GcString
            let gc_ref: GcRef<GcString> = unsafe { GcRef::from_ptr(obj as *const GcString) };
            string_pool.remove(gc_ref);
        }

        // 从弱表列表中移除（如果是 Table）
        if gc_type == GcObjectType::Table {
            self.weak_tables.retain(|&t| t != obj);
        }

        // 更新统计信息
        self.total_memory = self.total_memory.saturating_sub(obj_size);
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
            Self::free_gc_object(obj, gc_type);
        }
    }

    /// 获取 GC 对象的大小（用于统计更新）
    ///
    /// # Safety
    /// `obj` 必须指向有效的 GC 对象。
    unsafe fn object_size_of(&self, obj: *mut GcObjectHeader) -> usize {
        // SAFETY: caller guarantees obj is valid
        unsafe {
            match (*obj).gc_type() {
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

    #[test]
    fn test_sweep_removes_white_objects() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let obj1 = gc.create(GcString::new("keep"));
        let _obj2 = gc.create(GcString::new("sweep"));

        // obj1 标记为黑色（存活），obj2 保持白色（应被回收）
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
        unsafe {
            let header = survivor.as_ptr() as *mut GcObjectHeader;
            (*header).set_color(GcColor::Black);
        }

        gc.sweep(&mut pool);

        // sweep 后存活对象应重置为白色
        unsafe {
            let header = survivor.as_ptr() as *mut GcObjectHeader;
            assert!((*header).is_white());
        }
    }

    #[test]
    fn test_sweep_removes_string_from_pool() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let _s = pool.intern(&mut gc, "temporary_string");
        assert!(pool.find("temporary_string").is_some());

        // sweep（字符串为白色 → 应被回收）
        let _collected = gc.sweep(&mut pool);

        // 字符串应从池中移除
        assert!(pool.find("temporary_string").is_none());
    }

    #[test]
    fn test_sweep_empty_list() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 0);
    }
}

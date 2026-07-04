//! GC 弱表处理实现
//!
//! 弱表允许键和/或值被弱引用：当键或值仅被弱表引用时，GC 可以回收它们。
//! 在 sweep 之前清理弱表中的死亡条目。
//!
//! C++ 参考: `lua_cpp/src/gc/gc_weak.cpp`

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gc::collector::GarbageCollector;
use crate::gc::header::GcObjectHeader;
use crate::gc::header::bits;
use crate::table::Table;
use crate::value::Value;

impl GarbageCollector {
    /// 清理所有已标记弱表中的死亡键/值条目
    ///
    /// 遍历 `weak_tables` 列表中的每个弱表，根据其弱模式
    /// 删除指向已死对象的键和/或值。
    ///
    /// 必须在 sweep 删除白色对象之前调用，确保仍可安全检查键和值的颜色。
    ///
    /// C++ 对应: `GarbageCollector::clearWeakTableEntries()`
    pub fn clear_weak_table_entries(&mut self) {
        // 克隆列表以避免在清理期间修改时出现借用冲突。兼容路径可能在
        // 多次轻量 collect 后清空注册列表，所以同时从对象链补扫弱表位。
        let mut weak_tables: Vec<*mut GcObjectHeader> = self.weak_tables.clone();
        let mut current = self.all_objects;
        while !current.is_null() {
            // SAFETY: current walks the GC intrusive object list.
            unsafe {
                if (*current).gc_type() == crate::types::GcObjectType::Table
                    && ((*current).marked() & bits::WEAKBITS) != 0
                    && !weak_tables.contains(&current)
                {
                    weak_tables.push(current);
                }
                current = (*current).next();
            }
        }

        for &table_header in &weak_tables {
            if table_header.is_null() {
                continue;
            }

            // 读取弱模式标志位
            let (weak_keys, weak_values) = {
                // SAFETY: table_header is valid and not dead
                let marked = unsafe { (*table_header).marked() };
                let wk = (marked & bits::WEAKKEY) != 0;
                let wv = (marked & bits::WEAKVALUE) != 0;
                (wk, wv)
            };

            if !weak_keys && !weak_values {
                continue;
            }

            // 清理弱表中的死亡条目
            // SAFETY: table_header points to a valid Table
            unsafe {
                let table = &mut *(table_header as *mut Table);
                Self::remove_weak_entries_from_table(table, self, weak_keys, weak_values);
            }
        }
    }

    /// 从单个表中移除弱引用死亡条目
    ///
    /// C++ 对应: `Table::removeWeakEntries(const GarbageCollector& gc, bool weakKeys, bool weakValues)`
    fn remove_weak_entries_from_table(
        table: &mut Table,
        gc: &GarbageCollector,
        weak_keys: bool,
        weak_values: bool,
    ) {
        // 收集需要删除的键（避免在遍历时修改）
        let mut keys_to_remove: Vec<Value> = Vec::new();

        // 遍历所有键值对
        let mut key = Value::Nil;
        while let Some((k, v)) = table.next(&key) {
            let mut should_remove = false;

            let key_kept_alive_by_strong_value = weak_keys && !weak_values && k == v;
            let key_pending_finalizer = matches!(
                &k,
                Value::Userdata(userdata)
                    if gc
                        .pending_finalizers
                        .contains(&(userdata.as_ptr() as *mut GcObjectHeader))
            );
            let key_finalized_userdata = matches!(
                &k,
                Value::Userdata(userdata)
                    if unsafe {
                        (*(userdata.as_ptr() as *mut GcObjectHeader)).is_finalized()
                    } && !key_pending_finalizer
            );
            if weak_keys
                && (gc.is_value_dead(&k) || key_finalized_userdata)
                && !key_kept_alive_by_strong_value
                && !key_pending_finalizer
            {
                should_remove = true;
            }
            if weak_values && gc.is_weak_value_dead(&v) {
                should_remove = true;
            }

            if should_remove {
                keys_to_remove.push(k.clone());
            }

            key = k;
        }

        // 删除收集到的键
        for k in &keys_to_remove {
            table.remove(k);
        }

        // 清理数组中的弱值（仅弱值模式）
        if weak_values {
            Self::clear_weak_array_values(table, gc);
        }
    }

    /// 清理数组部分中的弱值死亡条目
    ///
    /// 在弱值模式下，将数组中指向已死对象的槽位置为 nil。
    fn clear_weak_array_values(table: &mut Table, gc: &GarbageCollector) {
        // 遍历数组索引
        for i in 1..=(table.array_size() as i32) {
            let val = table.get_array(i);
            if !val.is_nil() && gc.is_weak_value_dead(&val) {
                table.set_array(i, &Value::Nil);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc_string::GcString;
    use crate::table::Table;

    #[test]
    fn test_clear_weak_entries_empty_weak_tables() {
        let mut gc = GarbageCollector::new();
        // 没有弱表注册 → 不应 panic
        gc.clear_weak_table_entries();
    }

    #[test]
    fn test_is_object_dead_null() {
        let gc = GarbageCollector::new();
        assert!(!gc.is_object_dead(std::ptr::null_mut()));
    }

    #[test]
    fn test_is_value_dead_non_gc_types() {
        let gc = GarbageCollector::new();
        assert!(!gc.is_value_dead(&Value::Nil));
        assert!(!gc.is_value_dead(&Value::Boolean(true)));
        assert!(!gc.is_value_dead(&Value::Number(42.0)));
    }

    #[test]
    fn test_is_value_dead_string_always_alive() {
        let mut gc = GarbageCollector::new();
        let s = gc.create(GcString::new("alive"));
        // 字符串即使白色也被视为存活
        unsafe {
            let h = s.as_ptr() as *mut GcObjectHeader;
            (*h).set_color(crate::types::GcColor::White);
        }
        assert!(!gc.is_value_dead(&Value::String(s)));
    }

    #[test]
    fn test_is_value_dead_table() {
        let mut gc = GarbageCollector::new();
        let t = gc.create(Table::new());

        // 白色 → 死
        unsafe {
            let h = t.as_ptr() as *mut GcObjectHeader;
            (*h).set_color(crate::types::GcColor::White);
        }
        assert!(gc.is_value_dead(&Value::Table(t)));

        // 黑色 → 活
        unsafe {
            let h = t.as_ptr() as *mut GcObjectHeader;
            (*h).set_color(crate::types::GcColor::Black);
        }
        assert!(!gc.is_value_dead(&Value::Table(t)));
    }

    #[test]
    fn test_weak_table_marked_in_mark_phase() {
        let mut gc = GarbageCollector::new();

        // Create a root table with WEAKVALUE mode bit
        let table = gc.create_root(Table::new());
        let table_header = table.as_ptr() as *mut GcObjectHeader;

        unsafe {
            (*table_header).set_marked((*table_header).marked() | bits::WEAKVALUE);
        }
        gc.weak_tables.push(table_header);

        // Insert a child Table reference
        let child = gc.create(Table::new());
        let child_header = child.as_ptr() as *mut GcObjectHeader;

        // Make child white (dead)
        unsafe {
            (*child_header).set_color(crate::types::GcColor::White);
        }

        // Verify: dead white non-string value is detected
        assert!(gc.is_weak_value_dead(&Value::Table(child)));

        // Verify: dead white object is detected
        assert!(gc.is_object_dead(child_header));

        // Verify: alive (black) object is not dead
        unsafe {
            (*child_header).set_color(crate::types::GcColor::Black);
        }
        assert!(!gc.is_object_dead(child_header));
        assert!(!gc.is_weak_value_dead(&Value::Table(child)));
    }
}

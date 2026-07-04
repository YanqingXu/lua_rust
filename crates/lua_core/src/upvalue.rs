//! Upvalue：闭包捕获的外部变量
//!
//! Upvalue 是 Lua 实现闭包的关键数据结构。当内部函数引用外部函数的局部变量时，
//! 这些变量被"提升"为上值，在外部函数返回后仍保持可访问。
//!
//! ## 状态机
//! - **Open**：上值指向栈上的活跃变量（通过 stack_index + owner_stack 访问）
//! - **Closed**：上值拥有变量的独立副本（存储在 closed_value 中）
//!
//! 状态转换：Open → Closed（当外部函数返回，栈上的变量被销毁时）
//!
//! C++ 参考: `lua_cpp/src/core/upvalue.hpp`, `lua_cpp/src/core/upvalue.cpp`

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::types::GcObjectType;
use crate::value::Value;

/// Upvalue 对象 — GC 管理的闭包上值
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - is_open: bool (1 byte)
/// - stack_index: usize (8 bytes)
/// - closed_value: Value (16 bytes)
/// - next: Option<GcRef<Upvalue>> (8 bytes)
/// - owner_stack: *mut () (8 bytes — opaque, Phase 3 类型化为 Stack*)
///   总计约 57+ bytes
///
/// C++ 对应: `Lua::Upvalue`
#[repr(C)]
pub struct Upvalue {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// 是否为 Open 状态
    is_open: bool,

    /// 栈索引（Open: 栈位置；Closed: 保持原值用于调试）
    stack_index: usize,

    /// Closed 状态下的独立存储值
    closed_value: Value,

    /// 链表指针：LuaState 中 open upvalue 链表的 next 指针
    next: Option<GcRef<Upvalue>>,

    /// 所属栈指针（仅 Open 状态有效；Phase 3 类型化为 `*const Stack`）
    owner_stack: *mut std::ffi::c_void,
}

impl Upvalue {
    /// 创建 Open 状态的 Upvalue（指向栈上的值）
    ///
    /// `stack_index`: 栈索引位置
    /// `owner_stack`: 所属栈的不透明指针（Phase 3 类型化为 `&Stack`）
    ///
    /// C++ 对应: `Upvalue::createOpen(usize stackIndex, Stack& ownerStack)`
    pub fn new_open(stack_index: usize, owner_stack: *mut std::ffi::c_void) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Upval),
            is_open: true,
            stack_index,
            closed_value: Value::Nil,
            next: None,
            owner_stack,
        }
    }

    /// 创建 Closed 状态的 Upvalue（独立存储值）
    ///
    /// C++ 对应: `Upvalue::createClosed(const Value& value)`
    pub fn new_closed(value: Value) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Upval),
            is_open: false,
            stack_index: 0,
            closed_value: value,
            next: None,
            owner_stack: std::ptr::null_mut(),
        }
    }

    // ── 状态查询 ────────────────────────────────────────────────

    /// 检查是否为 Open 状态（指向栈上的值）
    #[inline]
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// 检查是否为 Closed 状态（拥有值的独立副本）
    #[inline]
    pub fn is_closed(&self) -> bool {
        !self.is_open
    }

    // ── 值访问 ──────────────────────────────────────────────────

    /// 获取 Closed 状态下的值（无需 Stack 引用）
    ///
    /// Phase 1.4: 仅提供 closed 访问。Open 状态的栈访问需要 Phase 3 Stack 实现。
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Open 状态则 panic。
    pub fn get_closed_value(&self) -> &Value {
        assert!(self.is_closed(), "get_closed_value called on open upvalue");
        &self.closed_value
    }

    /// 设置 Closed 状态下的值
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Open 状态则 panic。
    pub fn set_closed_value(&mut self, value: Value) {
        assert!(self.is_closed(), "set_closed_value called on open upvalue");
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, value)
        self.closed_value = value;
    }

    /// 获取 Open 状态下的栈索引
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Closed 状态则 panic。
    #[inline]
    pub fn stack_index(&self) -> usize {
        assert!(self.is_open(), "stack_index called on closed upvalue");
        self.stack_index
    }

    /// 安全获取栈索引（任何状态）
    #[inline]
    pub fn stack_index_any(&self) -> usize {
        self.stack_index
    }

    /// 获取 Open upvalue 所属栈的不透明指针。
    #[inline]
    pub fn owner_stack(&self) -> *mut std::ffi::c_void {
        self.owner_stack
    }

    // ── 状态转换 ────────────────────────────────────────────────

    /// 关闭 Upvalue（从 Open 转换为 Closed）
    ///
    /// 将栈上的值复制到内部存储，标记为 Closed。
    ///
    /// `stack_value`: 从栈上读取的当前值
    ///
    /// C++ 对应: `Upvalue::close(Stack& stack)`
    ///
    /// Phase 1.4: 接受显式的栈值参数（而非直接访问 Stack）。
    /// Phase 3 实现 Stack 后将改为通过 owner_stack 自动读取。
    pub fn close(&mut self, stack_value: Value) {
        if self.is_closed() {
            return; // 已经是 Closed 状态
        }

        // 复制栈上的值到内部存储
        self.closed_value = stack_value;
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, closedValue_)

        // 标记为 Closed
        self.is_open = false;
        self.owner_stack = std::ptr::null_mut();
        // stack_index 保持不变（用于调试）
    }

    // ── 链表管理 ────────────────────────────────────────────────

    /// 获取链表中的下一个 Upvalue
    #[inline]
    pub fn next(&self) -> Option<GcRef<Upvalue>> {
        self.next
    }

    /// 设置链表中的下一个 Upvalue
    #[inline]
    pub fn set_next(&mut self, next: Option<GcRef<Upvalue>>) {
        self.next = next;
    }
}

impl Default for Upvalue {
    fn default() -> Self {
        Self::new_closed(Value::Nil)
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Upvalue 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Upval。
// mark_children 标记 closed_value 中的 GC 对象。
unsafe impl GcObject for Upvalue {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记 Upvalue 引用的 GC 对象
    ///
    /// - Closed 状态：标记 closed_value 中的 GC 对象
    /// - Open 状态：栈上的值由栈管理，不在此标记
    ///
    /// C++ 对应: `Upvalue::mark(GarbageCollector& gc)`
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        if self.is_closed() {
            // SAFETY: collector is valid during mark phase
            Self::mark_value(&self.closed_value, collector);
        }
        // Open 状态：栈上的值由 LuaState 栈标记路径负责
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Upvalue {
    /// 标记 Value 中引用的 GC 对象（辅助方法）
    fn mark_value(val: &Value, collector: &mut GarbageCollector) {
        // SAFETY: all match arms dereference valid GcRef pointers;
        // collector is valid during mark phase.
        unsafe {
            match val {
                Value::String(s) => {
                    collector.mark_object(s.as_ptr() as *mut GcObjectHeader);
                }
                Value::Table(t) => {
                    collector.mark_object(t.as_ptr() as *mut GcObjectHeader);
                }
                Value::Function(f) => {
                    collector.mark_object(f.as_ptr() as *mut GcObjectHeader);
                }
                Value::Userdata(u) => {
                    collector.mark_object(u.as_ptr() as *mut GcObjectHeader);
                }
                Value::Thread(t) => {
                    collector.mark_object(t.as_ptr() as *mut GcObjectHeader);
                }
                _ => {}
            }
        }
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Upvalue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_open {
            f.debug_struct("Upvalue")
                .field("state", &"open")
                .field("stack_index", &self.stack_index)
                .finish()
        } else {
            f.debug_struct("Upvalue")
                .field("state", &"closed")
                .field("closed_value", &self.closed_value)
                .finish()
        }
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
    use crate::table::Table;

    // ── 创建测试 ────────────────────────────────────────────────

    #[test]
    fn test_create_open_upvalue() {
        let stack_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let uv = Upvalue::new_open(5, stack_ptr);

        assert!(uv.is_open());
        assert!(!uv.is_closed());
        assert_eq!(uv.stack_index(), 5);
        assert_eq!(uv.stack_index_any(), 5);
        assert!(uv.next().is_none());
    }

    #[test]
    fn test_create_closed_upvalue() {
        let uv = Upvalue::new_closed(Value::Number(42.0));

        assert!(!uv.is_open());
        assert!(uv.is_closed());
        assert_eq!(uv.stack_index_any(), 0);
        assert_eq!(*uv.get_closed_value(), Value::Number(42.0));
        assert!(uv.next().is_none());
    }

    #[test]
    fn test_default_upvalue() {
        let uv = Upvalue::default();
        assert!(uv.is_closed());
        assert_eq!(*uv.get_closed_value(), Value::Nil);
    }

    // ── Open 状态操作 ───────────────────────────────────────────

    #[test]
    fn test_open_upvalue_stack_index() {
        let uv = Upvalue::new_open(10, std::ptr::null_mut());
        assert_eq!(uv.stack_index(), 10);
    }

    #[test]
    #[should_panic(expected = "get_closed_value called on open upvalue")]
    fn test_get_closed_value_on_open_panics() {
        let uv = Upvalue::new_open(1, std::ptr::null_mut());
        uv.get_closed_value();
    }

    // ── 状态转换测试 ────────────────────────────────────────────

    #[test]
    fn test_close_upvalue() {
        let mut uv = Upvalue::new_open(3, std::ptr::null_mut());
        assert!(uv.is_open());

        // 关闭：将栈上的值（42.0）复制到内部存储
        uv.close(Value::Number(42.0));

        assert!(uv.is_closed());
        assert_eq!(*uv.get_closed_value(), Value::Number(42.0));
        // stack_index 保持不变（调试用）
        assert_eq!(uv.stack_index_any(), 3);
    }

    #[test]
    fn test_close_already_closed_is_noop() {
        let mut uv = Upvalue::new_closed(Value::Boolean(true));
        assert!(uv.is_closed());

        // 对已关闭的 upvalue 再次关闭应为空操作
        uv.close(Value::Number(99.0));
        assert!(uv.is_closed());
        assert_eq!(*uv.get_closed_value(), Value::Boolean(true));
    }

    // ── Closed 值操作 ───────────────────────────────────────────

    #[test]
    fn test_set_closed_value() {
        let mut uv = Upvalue::new_closed(Value::Nil);
        uv.set_closed_value(Value::Number(3.14));
        assert_eq!(*uv.get_closed_value(), Value::Number(3.14));
    }

    #[test]
    #[should_panic(expected = "set_closed_value called on open upvalue")]
    fn test_set_closed_value_on_open_panics() {
        let mut uv = Upvalue::new_open(1, std::ptr::null_mut());
        uv.set_closed_value(Value::Nil);
    }

    // ── 链表操作 ────────────────────────────────────────────────

    #[test]
    fn test_upvalue_linked_list() {
        let mut uv1 = Upvalue::new_closed(Value::Number(1.0));
        let uv2 = Upvalue::new_closed(Value::Number(2.0));

        // uv1 -> uv2
        let mut gc = GarbageCollector::new();
        let uv2_ref = gc.create(uv2);
        uv1.set_next(Some(uv2_ref));

        assert!(uv1.next().is_some());
        assert_eq!(uv1.next().unwrap(), uv2_ref);
    }

    // ── GC 类型测试 ─────────────────────────────────────────────

    #[test]
    fn test_upvalue_gc_header_type() {
        let uv = Upvalue::new_closed(Value::Nil);
        assert_eq!(uv.gc_header().gc_type(), GcObjectType::Upval);
    }

    #[test]
    fn test_upvalue_gc_create_and_register() {
        let mut gc = GarbageCollector::new();
        let uv = Upvalue::new_closed(Value::Number(42.0));
        let uv_ref: GcRef<Upvalue> = gc.create(uv);

        assert!(!uv_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    // ── GC 标记测试 ─────────────────────────────────────────────

    #[test]
    fn test_upvalue_mark_closed_with_gc_ref() {
        let mut gc = GarbageCollector::new();

        // 创建 Closed upvalue，包含一个 Table 引用
        let table_ref = gc.create(Table::new());
        let uv = Upvalue::new_closed(Value::Table(table_ref));
        let uv_ref = gc.create(uv);

        // 重置标记
        gc.reset_marks();

        // 标记 upvalue 的子对象
        // SAFETY: uv_ref is valid
        unsafe {
            let uv_ptr = uv_ref.as_ptr();
            (*uv_ptr).mark_children(&mut gc);
        }

        // Table 应被标记
        let table_header = table_ref.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*table_header).is_white(), "Table should be marked");
        }
    }

    #[test]
    fn test_upvalue_mark_open_marks_nothing() {
        let mut gc = GarbageCollector::new();

        // 创建 Open upvalue（栈上的值不由它标记）
        let uv = Upvalue::new_open(5, std::ptr::null_mut());
        let uv_ref = gc.create(uv);

        gc.reset_marks();

        // 标记 open upvalue 不应 panic
        unsafe {
            let uv_ptr = uv_ref.as_ptr();
            (*uv_ptr).mark_children(&mut gc);
        }
    }

    // ── Closed upvalue with GC string ────────────────────────────

    #[test]
    fn test_upvalue_closed_with_string() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s_ref = pool.intern(&mut gc, "captured");
        let uv = Upvalue::new_closed(Value::String(s_ref));
        let uv_ref = gc.create(uv);

        gc.reset_marks();

        // 标记应传播到字符串
        unsafe {
            let uv_ptr = uv_ref.as_ptr();
            (*uv_ptr).mark_children(&mut gc);
        }

        let s_header = s_ref.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*s_header).is_white(), "String should be marked");
        }
    }

    // ── GC 回收测试 ─────────────────────────────────────────────

    #[test]
    fn test_upvalue_swept_when_unreachable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 创建非根 Upvalue
        gc.create(Upvalue::new_closed(Value::Nil));
        assert_eq!(gc.object_count(), 1);

        // 标记：Upvalue 不是根 → 保持白色
        gc.mark();

        // 清扫：白色 Upvalue 应被回收
        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 0);
    }

    #[test]
    fn test_upvalue_kept_when_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 创建根 Upvalue
        gc.create_root(Upvalue::new_closed(Value::Boolean(true)));
        assert_eq!(gc.object_count(), 1);

        // 完整 GC 循环
        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 1);
    }

    // ── get_size 测试 ───────────────────────────────────────────

    #[test]
    fn test_upvalue_get_size() {
        let uv = Upvalue::new_closed(Value::Nil);
        let size = uv.get_size();
        assert!(size >= std::mem::size_of::<Upvalue>());
    }

    // ── Debug 输出 ──────────────────────────────────────────────

    #[test]
    fn test_upvalue_debug_open() {
        let uv = Upvalue::new_open(7, std::ptr::null_mut());
        let debug_str = format!("{:?}", uv);
        assert!(debug_str.contains("open"));
        assert!(debug_str.contains("7"));
    }

    #[test]
    fn test_upvalue_debug_closed() {
        let uv = Upvalue::new_closed(Value::Number(1.5));
        let debug_str = format!("{:?}", uv);
        assert!(debug_str.contains("closed"));
    }
}

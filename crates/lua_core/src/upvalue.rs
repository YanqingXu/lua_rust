//! Upvalue：闭包捕获的外部变量
//!
//! Upvalue 是 Lua 实现闭包的关键数据结构。当内部函数引用外部函数的局部变量时，
//! 这些变量被"提升"为上值，在外部函数返回后仍保持可访问。
//!
//! ## 状态机
//! - **Open**：上值通过 `StateHandle + stack index` 标识栈上的活跃变量
//! - **Closed**：上值拥有变量的独立副本（存储在 closed_value 中）
//!
//! 状态转换：Open → Closed（当外部函数返回，栈上的变量被销毁时）
//!

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
use crate::state_handle::StateHandle;
use crate::types::GcObjectType;
use crate::value::Value;

#[derive(Clone, Debug)]
enum UpvalueState {
    Open {
        owner: StateHandle,
        stack_index: usize,
    },
    Closed {
        value: Value,
        former_stack_index: Option<usize>,
    },
}

/// Upvalue 对象 — GC 管理的闭包上值
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - state: Open (`StateHandle + stack index`) or Closed (`Value`)
///
#[repr(C)]
pub struct Upvalue {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    state: UpvalueState,
}

impl Upvalue {
    /// 创建 Open 状态的 Upvalue（指向栈上的值）
    ///
    /// `owner`: 所属 `LuaState` 的 runtime-scoped generational handle
    /// `stack_index`: 栈索引位置
    pub fn new_open(owner: StateHandle, stack_index: usize) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Upval),
            state: UpvalueState::Open { owner, stack_index },
        }
    }

    /// 创建 Closed 状态的 Upvalue（独立存储值）
    ///
    pub fn new_closed(value: Value) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Upval),
            state: UpvalueState::Closed {
                value,
                former_stack_index: None,
            },
        }
    }

    // ── 状态查询 ────────────────────────────────────────────────

    /// 检查是否为 Open 状态（指向栈上的值）
    #[inline]
    pub fn is_open(&self) -> bool {
        matches!(&self.state, UpvalueState::Open { .. })
    }

    /// 检查是否为 Closed 状态（拥有值的独立副本）
    #[inline]
    pub fn is_closed(&self) -> bool {
        matches!(&self.state, UpvalueState::Closed { .. })
    }

    // ── 值访问 ──────────────────────────────────────────────────

    /// 获取 Closed 状态下的值（无需 Stack 引用）
    ///
    /// Phase 1.4: 仅提供 closed 访问。Open 状态的栈访问需要 Phase 3 Stack 实现。
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Open 状态则 panic。
    pub fn get_closed_value(&self) -> &Value {
        let UpvalueState::Closed { value, .. } = &self.state else {
            panic!("get_closed_value called on open upvalue");
        };
        value
    }

    /// 设置 Closed 状态下的值
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Open 状态则 panic。
    pub fn set_closed_value(&mut self, value: Value) {
        let UpvalueState::Closed {
            value: closed_value,
            ..
        } = &mut self.state
        else {
            panic!("set_closed_value called on open upvalue");
        };
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, value)
        *closed_value = value;
    }

    /// 获取 Open 状态下的栈索引
    ///
    /// # Panics
    /// 如果 Upvalue 处于 Closed 状态则 panic。
    #[inline]
    pub fn stack_index(&self) -> usize {
        let UpvalueState::Open { stack_index, .. } = &self.state else {
            panic!("stack_index called on closed upvalue");
        };
        *stack_index
    }

    /// 安全获取栈索引（任何状态）
    #[inline]
    pub fn stack_index_any(&self) -> usize {
        match &self.state {
            UpvalueState::Open { stack_index, .. } => *stack_index,
            UpvalueState::Closed {
                former_stack_index, ..
            } => former_stack_index.unwrap_or(0),
        }
    }

    /// Return the checked owner and stack slot of an open Upvalue.
    #[inline]
    pub fn open_location(&self) -> Option<(StateHandle, usize)> {
        match &self.state {
            UpvalueState::Open { owner, stack_index } => Some((*owner, *stack_index)),
            UpvalueState::Closed { .. } => None,
        }
    }

    // ── 状态转换 ────────────────────────────────────────────────

    /// 关闭 Upvalue（从 Open 转换为 Closed）
    ///
    /// 将栈上的值复制到内部存储，标记为 Closed。
    ///
    /// `stack_value`: 从栈上读取的当前值
    ///
    ///
    /// Runtime resolves the open location before calling this method.
    pub fn close(&mut self, stack_value: Value) {
        let stack_index = match &self.state {
            UpvalueState::Open { stack_index, .. } => *stack_index,
            UpvalueState::Closed { .. } => return,
        };
        self.state = UpvalueState::Closed {
            value: stack_value,
            former_stack_index: Some(stack_index),
        };
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, closedValue_)
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
    /// - Open 状态：栈上的值由状态根追踪器标记
    ///
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        if let UpvalueState::Closed { value, .. } = &self.state {
            Self::mark_value(value, collector);
        }
        // Open 状态的栈值由 LuaState 活跃窗口标记路径负责。
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Upvalue {
    /// 标记 Value 中引用的 GC 对象（辅助方法）
    fn mark_value(val: &Value, collector: &mut GarbageCollector) {
        collector.mark_registered_value(val);
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Upvalue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.state {
            UpvalueState::Open { owner, stack_index } => f
                .debug_struct("Upvalue")
                .field("state", &"open")
                .field("owner", owner)
                .field("stack_index", stack_index)
                .finish(),
            UpvalueState::Closed { value, .. } => f
                .debug_struct("Upvalue")
                .field("state", &"closed")
                .field("closed_value", value)
                .finish(),
        }
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::approx_constant)]

    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_ref::GcRef;
    use crate::state_handle::StateHandleIssuer;
    use crate::string_pool::StringPool;
    use crate::table::Table;
    use std::num::NonZeroU64;

    fn state_handle(slot: usize) -> StateHandle {
        StateHandleIssuer::try_new()
            .expect("test runtime namespace")
            .issue(slot, NonZeroU64::MIN)
    }

    // ── 创建测试 ────────────────────────────────────────────────

    #[test]
    fn test_create_open_upvalue() {
        let owner = state_handle(0);
        let uv = Upvalue::new_open(owner, 5);

        assert!(uv.is_open());
        assert!(!uv.is_closed());
        assert_eq!(uv.stack_index(), 5);
        assert_eq!(uv.stack_index_any(), 5);
        assert_eq!(uv.open_location(), Some((owner, 5)));
    }

    #[test]
    fn test_create_closed_upvalue() {
        let uv = Upvalue::new_closed(Value::Number(42.0));

        assert!(!uv.is_open());
        assert!(uv.is_closed());
        assert_eq!(uv.stack_index_any(), 0);
        assert_eq!(*uv.get_closed_value(), Value::Number(42.0));
        assert_eq!(uv.open_location(), None);
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
        let uv = Upvalue::new_open(state_handle(0), 10);
        assert_eq!(uv.stack_index(), 10);
    }

    #[test]
    #[should_panic(expected = "get_closed_value called on open upvalue")]
    fn test_get_closed_value_on_open_panics() {
        let uv = Upvalue::new_open(state_handle(0), 1);
        uv.get_closed_value();
    }

    // ── 状态转换测试 ────────────────────────────────────────────

    #[test]
    fn test_close_upvalue() {
        let mut uv = Upvalue::new_open(state_handle(0), 3);
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
        let mut uv = Upvalue::new_open(state_handle(0), 1);
        uv.set_closed_value(Value::Nil);
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
        // SAFETY: `uv_ref` is live and registered with `gc`, and that same
        // collector is exclusively borrowed for child marking.
        unsafe {
            let uv_ptr = uv_ref.as_ptr();
            (*uv_ptr).mark_children(&mut gc);
        }

        // Table 应被标记
        let table_header = table_ref.as_ptr() as *mut GcObjectHeader;
        // SAFETY: `table_ref` remains registered with `gc`; marking does not
        // release or relocate the table.
        unsafe {
            assert!(!(*table_header).is_white(), "Table should be marked");
        }
    }

    #[test]
    fn test_upvalue_mark_open_marks_nothing() {
        let mut gc = GarbageCollector::new();

        // 创建 Open upvalue（栈上的值不由它标记）
        let uv = Upvalue::new_open(state_handle(0), 5);
        let uv_ref = gc.create(uv);

        gc.reset_marks();

        // 标记 open upvalue 不应 panic
        // SAFETY: `uv_ref` is live in the exclusively borrowed collector, and
        // the open upvalue has no child pointer to dereference.
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

        let s_ref = pool.intern_bytes(&mut gc, b"captured");
        let uv = Upvalue::new_closed(Value::String(s_ref));
        let uv_ref = gc.create(uv);

        gc.reset_marks();

        // 标记应传播到字符串
        // SAFETY: `uv_ref` is a live registered upvalue and `gc` owns the
        // interned string referenced by it.
        unsafe {
            let uv_ptr = uv_ref.as_ptr();
            (*uv_ptr).mark_children(&mut gc);
        }

        let s_header = s_ref.as_ptr() as *mut GcObjectHeader;
        // SAFETY: `s_ref` remains interned and registered with `gc`; the mark
        // operation only updates its header bits.
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
        let uv = Upvalue::new_open(state_handle(0), 7);
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

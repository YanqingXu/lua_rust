//! GC 对象 trait
//!
//! 所有可被 GC 管理的对象必须实现 `GcObject` trait。
//! 为 GC 管理对象提供统一的标记和大小查询接口。
//!
//! # Safety
//!
//! `GcObject` 是 unsafe trait，因为实现者必须保证：
//! 1. 内存布局以 `GcObjectHeader` 开头
//! 2. `gc_header()` 返回有效的 header 引用
//! 3. `mark_children()` 正确报告所有引用关系
//!

use crate::gc::header::GcObjectHeader;
use crate::types::GcObjectType;

// GarbageCollector 在 collector 模块中定义
use super::collector::GarbageCollector;

// Keep the tag-based mark/sweep dispatcher closed over the seven concrete
// object layouts that it actually knows how to cast and destroy. In
// particular, an arbitrary downstream `GcObject` implementation must not be
// able to claim (for example) `GcObjectType::Thread` while having a different
// layout.
//
// TODO(M1 GC metadata): replace this closed dispatch contract with per-
// allocation type-erased metadata (trace, size and drop functions). Only then
// can custom GC object implementations be supported without concrete casts.
mod sealed {
    use crate::function::Function;
    use crate::gc_string::GcString;
    use crate::proto::Proto;
    use crate::table::Table;
    use crate::thread::Thread;
    use crate::types::GcObjectType;
    use crate::upvalue::Upvalue;
    use crate::userdata::Userdata;

    pub trait Sealed {
        const GC_TYPE: GcObjectType;
    }

    impl Sealed for GcString {
        const GC_TYPE: GcObjectType = GcObjectType::String;
    }

    impl Sealed for Table {
        const GC_TYPE: GcObjectType = GcObjectType::Table;
    }

    impl Sealed for Function {
        const GC_TYPE: GcObjectType = GcObjectType::Function;
    }

    impl Sealed for Userdata {
        const GC_TYPE: GcObjectType = GcObjectType::Userdata;
    }

    impl Sealed for Thread {
        const GC_TYPE: GcObjectType = GcObjectType::Thread;
    }

    impl Sealed for Proto {
        const GC_TYPE: GcObjectType = GcObjectType::Proto;
    }

    impl Sealed for Upvalue {
        const GC_TYPE: GcObjectType = GcObjectType::Upval;
    }
}

/// GC 可管理对象的 unsafe trait
///
/// 实现者必须在结构体开头包含 `GcObjectHeader`，
/// 并通过 `gc_header()` 返回其引用。
///
/// This trait is sealed because the collector currently dispatches tracing,
/// sizing and destruction by `GcObjectType` and concrete pointer casts. The
/// only valid implementations are the built-in Lua object layouts.
///
/// # Safety
///
/// 实现者必须确保:
/// - `gc_header()` 始终返回同一个 header 的有效引用
/// - header 的 `gc_type` 与实际对象类型匹配
/// - `mark_children()` 遍历并标记所有被该对象引用的 GC 对象
/// - `get_size()` 返回准确的对象总大小（字节）
pub unsafe trait GcObject: Sized + sealed::Sealed {
    /// Return the concrete tag expected by the collector's closed dispatcher.
    ///
    /// This is used by `GarbageCollector::create` to validate the layout/tag
    /// invariant before it registers an allocation.
    #[doc(hidden)]
    fn expected_gc_type() -> GcObjectType {
        <Self as sealed::Sealed>::GC_TYPE
    }

    /// 返回 GC 对象头部的引用
    fn gc_header(&self) -> &GcObjectHeader;

    /// 返回 GC 对象头部的可变引用（仅供 GC 内部使用）
    fn gc_header_mut(&mut self) -> &mut GcObjectHeader;

    /// 标记该对象引用的所有其他 GC 对象
    ///
    /// 在 GC 标记阶段调用。实现者应遍历所有 GC 引用，
    /// 对每个被引用的对象调用 `collector.mark_registered()` 或
    /// `collector.mark_registered_value()`，使 foreign/stale/type-confused
    /// child edge 在读取 header 前被拒绝。
    ///
    /// # Safety
    ///
    /// 调用者保证 collector 在标记期间保持有效。
    unsafe fn mark_children(&self, collector: &mut GarbageCollector);

    /// 获取对象占用的总内存大小（字节）
    fn get_size(&self) -> usize;
}

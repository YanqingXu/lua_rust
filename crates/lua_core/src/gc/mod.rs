//! GC 模块入口
//!
//! 垃圾回收基础设施：侵入式链表、GcRef 安全句柄、GcObject trait、
//! GarbageCollector 核心和可插拔策略。

pub mod collector;
pub mod gc_object;
pub mod gc_ref;
pub mod header;
pub mod strategy;
